//! AST → bytecode.
//!
//! Name resolution happens here, following dog3's assignment rule: `x = v`
//! writes to the nearest visible `x` (block local, enclosing function via a
//! closure, or module global) and only declares a new local when there is none.
//!
//! Every local gets its own slot. When a closure captures a local or someone
//! takes `&x`, the local is *boxed* into a cell; since that can be discovered
//! after code using the slot was emitted, the function is patched when it is
//! finished (`LoadLocal(s)` → `LoadCell(c)`, …).

use std::rc::Rc;

use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};

use super::bytecode::*;
use crate::error::{Error, ErrorKind, Result};
use crate::syntax::ast::*;
use crate::syntax::{SourceId, Span};
use crate::value::{UpdateOp, Value};
use crate::vm::{SigParam, Signature};

/// The globals of one module, shared by all code compiled into it (including `eval`).
#[derive(Debug, Default, Clone)]
pub struct ModuleScope {
    names: Vec<Rc<str>>,
    index: FxHashMap<Rc<str>, u32>,
}

impl ModuleScope {
    /// The slot of global `name`, creating it if needed.
    pub fn global(&mut self, name: &Rc<str>) -> u32 {
        if let Some(&i) = self.index.get(name) {
            return i;
        }
        let i = self.names.len() as u32;
        self.names.push(name.clone());
        self.index.insert(name.clone(), i);
        i
    }

    /// The slot of global `name`, if it exists.
    pub fn get(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// All global names, by slot.
    pub fn names(&self) -> &[Rc<str>] {
        &self.names
    }
}

/// Names the compiler can resolve to prelude values.
pub type PreludeNames = FxHashMap<Rc<str>, u32>;

/// Compiles a module's top level. New globals are added to `scope`.
pub fn compile_module(
    body: &Block,
    prelude: &PreludeNames,
    scope: &mut ModuleScope,
    source: SourceId,
) -> Result<Rc<Proto>> {
    let mut compiler = Compiler {
        prelude,
        module: scope,
        funcs: Vec::new(),
        source,
        span: body.span,
    };
    for stmt in &body.stmts {
        let name = match &stmt.kind {
            StmtKind::Assign { target, .. } => Some(&target.root),
            StmtKind::Let { name, .. } | StmtKind::Mod { name, .. } => Some(name),
            StmtKind::Fn(decl) => Some(&decl.name),
            StmtKind::Use { alias, .. } => Some(alias),
            _ => None,
        };
        if let Some(name) = name {
            compiler.module.global(name);
        }
    }
    let sig = Rc::new(Signature::new(Vec::new(), None));
    compiler
        .funcs
        .push(FnState::new("<main>".into(), true, sig, "".into()));
    compiler.open_scope();
    compiler.block_body(&body.stmts)?;
    compiler.emit(Op::Return);
    let state = compiler.funcs.pop().unwrap_or_else(|| unreachable!());
    Ok(Rc::new(state.finish(source)))
}

/// What a name resolved to.
#[derive(Clone, Copy)]
enum Resolved {
    Local(u16),
    Upval(u16),
    Global(u32),
    Prelude(u32),
}

/// Where a new declaration lives.
enum Target {
    Slot(u16),
    Global(u32),
}

struct LoopCtx {
    /// `Break` instructions waiting for the exit address.
    breaks: Vec<usize>,
    /// `Continue` instructions waiting for the continue address.
    continues: Vec<usize>,
}

/// Per-function compilation state.
struct FnState {
    name: Rc<str>,
    is_main: bool,
    sig: Rc<Signature>,
    source_text: Rc<str>,
    code: Vec<Op>,
    spans: Vec<Span>,
    consts: Vec<Value>,
    str_consts: FxHashMap<Rc<str>, u32>,
    protos: Vec<Rc<Proto>>,
    modules: Vec<Rc<ModuleProto>>,
    scopes: Vec<Vec<(Ident, u16)>>,
    /// For each slot, the cell it was boxed into.
    slot_cells: Vec<Option<u16>>,
    n_cells: u16,
    upvals: Vec<Capture>,
    loops: Vec<LoopCtx>,
    /// `StoreLocal`s that declare a variable (they become `DeclCell` when boxed).
    decls: FxHashSet<usize>,
    /// Placeholder `Nop`s for hoisted functions (become `NewCell` when boxed).
    hoists: Vec<(u16, usize)>,
    /// Placeholder `Nop`s for parameters (become `BoxParam` when boxed).
    prologue: Vec<(u16, usize)>,
}

impl FnState {
    fn new(name: Rc<str>, is_main: bool, sig: Rc<Signature>, source_text: Rc<str>) -> FnState {
        FnState {
            name,
            is_main,
            sig,
            source_text,
            code: Vec::new(),
            spans: Vec::new(),
            consts: Vec::new(),
            str_consts: FxHashMap::default(),
            protos: Vec::new(),
            modules: Vec::new(),
            scopes: Vec::new(),
            slot_cells: Vec::new(),
            n_cells: 0,
            upvals: Vec::new(),
            loops: Vec::new(),
            decls: FxHashSet::default(),
            hoists: Vec::new(),
            prologue: Vec::new(),
        }
    }

    fn lookup(&self, name: &str) -> Option<u16> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|(n, _)| &**n == name)
            .map(|(_, slot)| *slot)
    }

    /// The cell for `slot`, boxing it if it isn't yet.
    fn cell_for(&mut self, slot: u16) -> u16 {
        if let Some(cell) = self.slot_cells[slot as usize] {
            return cell;
        }
        let cell = self.n_cells;
        self.n_cells += 1;
        self.slot_cells[slot as usize] = Some(cell);
        cell
    }

    fn add_upval(&mut self, capture: Capture) -> u16 {
        if let Some(i) = self.upvals.iter().position(|c| *c == capture) {
            return i as u16;
        }
        self.upvals.push(capture);
        (self.upvals.len() - 1) as u16
    }

    /// Rewrites slot accesses of boxed locals into cell accesses and builds the proto.
    fn finish(mut self, source: SourceId) -> Proto {
        let cells = &self.slot_cells;
        let cell = |slot: u16| cells[slot as usize];
        for (i, op) in self.code.iter_mut().enumerate() {
            *op = match *op {
                Op::LoadLocal(s) => cell(s).map_or(*op, Op::LoadCell),
                Op::StoreLocal(s) => match cell(s) {
                    Some(c) if self.decls.contains(&i) => Op::DeclCell(c),
                    Some(c) => Op::StoreCell(c),
                    None => *op,
                },
                Op::Update {
                    var,
                    keys,
                    op: update,
                } => match var.get() {
                    Var::Local(s) => match cell(s) {
                        Some(c) => Op::Update {
                            var: VarRef::new(Var::Cell(c)),
                            keys,
                            op: update,
                        },
                        None => *op,
                    },
                    _ => *op,
                },
                other => other,
            };
        }
        for &(slot, at) in &self.hoists {
            if let Some(c) = cell(slot) {
                self.code[at] = Op::NewCell(c);
            }
        }
        for &(slot, at) in &self.prologue {
            if let Some(c) = cell(slot) {
                self.code[at] = Op::BoxParam { slot, cell: c };
            }
        }
        Proto {
            name: self.name,
            code: self.code.into_boxed_slice(),
            spans: self.spans.into_boxed_slice(),
            consts: self.consts.into_boxed_slice(),
            protos: self.protos.into_boxed_slice(),
            modules: self.modules.into_boxed_slice(),
            n_slots: self.slot_cells.len() as u16,
            n_cells: self.n_cells,
            captures: self.upvals.into_boxed_slice(),
            sig: self.sig,
            source_text: self.source_text,
            source,
        }
    }
}

struct Compiler<'a> {
    prelude: &'a PreludeNames,
    module: &'a mut ModuleScope,
    funcs: Vec<FnState>,
    source: SourceId,
    span: Span,
}

impl Compiler<'_> {
    // ----- emission ----------------------------------------------------------------

    fn f(&mut self) -> &mut FnState {
        self.funcs.last_mut().unwrap_or_else(|| unreachable!())
    }

    fn emit(&mut self, op: Op) -> usize {
        let span = self.span;
        let f = self.f();
        f.code.push(op);
        f.spans.push(span);
        f.code.len() - 1
    }

    fn here(&mut self) -> usize {
        self.f().code.len()
    }

    /// Emits a jump-like op with a placeholder offset, to be patched later.
    fn emit_jump(&mut self, make: fn(i32) -> Op) -> usize {
        self.emit(make(0))
    }

    /// Points the jump at `at` to the current position.
    fn patch_jump(&mut self, at: usize) {
        let target = self.here();
        self.patch_jump_to(at, target);
    }

    fn patch_jump_to(&mut self, at: usize, target: usize) {
        let offset = target as i32 - at as i32 - 1;
        let op = &mut self.f().code[at];
        *op = match *op {
            Op::Jump(_) => Op::Jump(offset),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(offset),
            Op::JumpIfFalsyKeep(_) => Op::JumpIfFalsyKeep(offset),
            Op::JumpIfTruthyKeep(_) => Op::JumpIfTruthyKeep(offset),
            Op::JumpIfNotNullKeep(_) => Op::JumpIfNotNullKeep(offset),
            Op::Break(_) => Op::Break(offset),
            Op::Continue(_) => Op::Continue(offset),
            Op::PushHandler(_) => Op::PushHandler(offset),
            Op::IterNext { pairs, .. } => Op::IterNext {
                pairs,
                exit: offset,
            },
            other => other,
        };
    }

    /// Emits a backwards jump to `target`.
    fn emit_loop(&mut self, target: usize) {
        let at = self.here();
        self.emit(Op::Jump(target as i32 - at as i32 - 1));
    }

    fn constant(&mut self, value: Value) -> Result<u32> {
        let f = self.f();
        if let Value::Str(s) = &value {
            if let Some(&i) = f.str_consts.get(s.as_str()) {
                return Ok(i);
            }
        }
        let i = u32::try_from(f.consts.len()).map_err(|_| self.too_big("constants"))?;
        let f = self.f();
        if let Value::Str(s) = &value {
            f.str_consts.insert(Rc::from(s.as_str()), i);
        }
        f.consts.push(value);
        Ok(i)
    }

    fn error(&self, message: impl Into<String>, span: Span) -> Error {
        Error::new(ErrorKind::Name, message).at(self.source, span)
    }

    fn too_big(&self, what: &str) -> Error {
        Error::new(
            ErrorKind::Syntax,
            format!("this function has too many {what}"),
        )
        .at(self.source, self.span)
    }

    // ----- scopes and names ------------------------------------------------------------

    fn open_scope(&mut self) {
        self.f().scopes.push(Vec::new());
    }

    fn close_scope(&mut self) {
        self.f().scopes.pop();
    }

    fn at_module_top(&mut self) -> bool {
        let f = self.f();
        f.is_main && f.scopes.len() == 1
    }

    fn new_slot(&mut self, name: &Ident) -> Result<u16> {
        let slot =
            u16::try_from(self.f().slot_cells.len()).map_err(|_| self.too_big("variables"))?;
        let f = self.f();
        f.slot_cells.push(None);
        f.scopes
            .last_mut()
            .unwrap_or_else(|| unreachable!())
            .push((name.clone(), slot));
        Ok(slot)
    }

    /// Declares `name` in the current scope.
    fn declare(&mut self, name: &Ident) -> Result<Target> {
        if self.at_module_top() {
            Ok(Target::Global(self.module.global(name)))
        } else {
            Ok(Target::Slot(self.new_slot(name)?))
        }
    }

    fn resolve(&mut self, name: &str) -> Option<Resolved> {
        let top = self.funcs.len() - 1;
        if let Some(slot) = self.funcs[top].lookup(name) {
            return Some(Resolved::Local(slot));
        }
        if let Some(upval) = self.resolve_upval(top, name) {
            return Some(Resolved::Upval(upval));
        }
        if let Some(global) = self.module.get(name) {
            return Some(Resolved::Global(global));
        }
        self.prelude.get(name).map(|&p| Resolved::Prelude(p))
    }

    fn resolve_upval(&mut self, func: usize, name: &str) -> Option<u16> {
        if func == 0 {
            return None;
        }
        let parent = func - 1;
        let capture = match self.funcs[parent].lookup(name) {
            Some(slot) => Capture::Cell(self.funcs[parent].cell_for(slot)),
            None => Capture::Upval(self.resolve_upval(parent, name)?),
        };
        Some(self.funcs[func].add_upval(capture))
    }

    /// Resolves a name that is being assigned to (the prelude is read-only).
    fn resolve_var(&mut self, name: &str) -> Option<Var> {
        match self.resolve(name)? {
            Resolved::Local(slot) => Some(match self.f().slot_cells[slot as usize] {
                Some(cell) => Var::Cell(cell),
                None => Var::Local(slot),
            }),
            Resolved::Upval(u) => Some(Var::Upval(u)),
            Resolved::Global(g) => Some(Var::Global(g)),
            Resolved::Prelude(_) => None,
        }
    }

    fn load(&mut self, name: &Ident, span: Span) -> Result<()> {
        match self.resolve(name) {
            Some(Resolved::Local(slot)) => self.emit(Op::LoadLocal(slot)),
            Some(Resolved::Upval(u)) => self.emit(Op::LoadUpval(u)),
            Some(Resolved::Global(g)) => self.emit(Op::LoadGlobal(g)),
            Some(Resolved::Prelude(p)) => self.emit(Op::LoadPrelude(p)),
            None => return Err(self.error(format!("`{name}` is not defined"), span)),
        };
        Ok(())
    }

    fn store_var(&mut self, var: Var) {
        match var {
            Var::Local(slot) => self.emit(Op::StoreLocal(slot)),
            Var::Cell(cell) => self.emit(Op::StoreCell(cell)),
            Var::Upval(u) => self.emit(Op::StoreUpval(u)),
            Var::Global(g) => self.emit(Op::StoreGlobal(g)),
        };
    }

    /// Stores the top of the stack into a freshly declared target.
    fn store_new(&mut self, target: Target) {
        match target {
            Target::Global(g) => {
                self.emit(Op::StoreGlobal(g));
            }
            Target::Slot(slot) => {
                let at = self.emit(Op::StoreLocal(slot));
                self.f().decls.insert(at);
            }
        }
    }

    // ----- blocks and statements --------------------------------------------------------

    /// Hoists the block's functions, then compiles its statements into one accumulator.
    fn block_body(&mut self, stmts: &[Stmt]) -> Result<()> {
        self.hoist_fns(stmts)?;
        self.emit(Op::AccBegin);
        for stmt in stmts {
            self.stmt(stmt)?;
        }
        self.emit(Op::AccEnd);
        Ok(())
    }

    fn block(&mut self, block: &Block) -> Result<()> {
        self.open_scope();
        self.block_body(&block.stmts)?;
        self.close_scope();
        Ok(())
    }

    /// Creates every function declared directly in this block before any statement
    /// runs, so they can call each other regardless of order. Several `fn`s with the
    /// same name form one overload set.
    fn hoist_fns(&mut self, stmts: &[Stmt]) -> Result<()> {
        let mut groups: IndexMap<Ident, Vec<&Rc<FnDecl>>> = IndexMap::new();
        for stmt in stmts {
            if let StmtKind::Fn(decl) = &stmt.kind {
                groups.entry(decl.name.clone()).or_default().push(decl);
            }
        }
        if groups.is_empty() {
            return Ok(());
        }
        let mut targets = Vec::with_capacity(groups.len());
        for name in groups.keys() {
            let target = self.declare(name)?;
            if let Target::Slot(slot) = target {
                let at = self.emit(Op::Nop);
                self.f().hoists.push((slot, at));
            }
            targets.push(target);
        }
        for ((_, decls), target) in groups.into_iter().zip(targets) {
            for (i, a) in decls.iter().enumerate() {
                for b in &decls[..i] {
                    if a.sig_key() == b.sig_key() {
                        return Err(Error::new(
                            ErrorKind::Syntax,
                            format!("`{}` is already defined with the same parameters", a.name),
                        )
                        .at(self.source, a.span));
                    }
                }
            }
            for decl in &decls {
                self.span = decl.span;
                self.function(decl)?;
            }
            if decls.len() > 1 {
                self.emit(Op::MakeOverloads(decls.len() as u16));
            }
            match target {
                Target::Global(g) => self.emit(Op::StoreGlobal(g)),
                Target::Slot(slot) => self.emit(Op::StoreLocal(slot)),
            };
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<()> {
        self.span = stmt.span;
        match &stmt.kind {
            StmtKind::Output(expr) => {
                self.expr(expr)?;
                self.span = stmt.span;
                self.emit(Op::AccPut);
            }
            StmtKind::Discard(expr) => {
                self.expr(expr)?;
                self.emit(Op::Pop);
            }
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value)?,
            StmtKind::Let { name, value } => match value {
                LetValue::Value(value) => {
                    self.expr(value)?;
                    let target = self.declare(name)?;
                    self.store_new(target);
                }
                LetValue::Ref(place) => {
                    if self.at_module_top() {
                        return Err(Error::new(
                            ErrorKind::Syntax,
                            "references can only be created inside a block or function",
                        )
                        .at(self.source, stmt.span));
                    }
                    self.make_ref(place)?;
                    let slot = self.new_slot(name)?;
                    let cell = self.f().cell_for(slot);
                    self.emit(Op::DeclRef(cell));
                }
            },
            StmtKind::Fn(_) => {}
            StmtKind::Mod { name, body } => {
                let mut scope = ModuleScope::default();
                let main = compile_module(body, self.prelude, &mut scope, self.source)?;
                let module = Rc::new(ModuleProto {
                    main,
                    globals: scope.names,
                });
                let f = self.f();
                f.modules.push(module);
                let index = (f.modules.len() - 1) as u32;
                self.span = stmt.span;
                self.emit(Op::MakeModule(index));
                let target = self.declare(name)?;
                self.store_new(target);
            }
            StmtKind::Use { path, alias } => {
                self.expr(path)?;
                self.span = stmt.span;
                self.emit(Op::Use);
                let target = self.declare(alias)?;
                self.store_new(target);
            }
            StmtKind::Return(value) => {
                self.opt_expr(value.as_ref())?;
                self.emit(Op::Return);
            }
            StmtKind::Clear(value) => {
                self.opt_expr(value.as_ref())?;
                self.emit(Op::AccClear);
            }
            StmtKind::Break(value) => {
                if self.f().loops.is_empty() {
                    return Err(Error::new(ErrorKind::Syntax, "`break` outside a loop")
                        .at(self.source, stmt.span));
                }
                self.opt_expr(value.as_ref())?;
                let at = self.emit_jump(Op::Break);
                self.f()
                    .loops
                    .last_mut()
                    .unwrap_or_else(|| unreachable!())
                    .breaks
                    .push(at);
            }
            StmtKind::Continue => {
                if self.f().loops.is_empty() {
                    return Err(Error::new(ErrorKind::Syntax, "`continue` outside a loop")
                        .at(self.source, stmt.span));
                }
                let at = self.emit_jump(Op::Continue);
                self.f()
                    .loops
                    .last_mut()
                    .unwrap_or_else(|| unreachable!())
                    .continues
                    .push(at);
            }
            StmtKind::Throw(value) => {
                self.expr(value)?;
                self.span = stmt.span;
                self.emit(Op::Throw);
            }
        }
        Ok(())
    }

    fn opt_expr(&mut self, expr: Option<&Expr>) -> Result<()> {
        match expr {
            Some(expr) => self.expr(expr),
            None => {
                self.emit(Op::Null);
                Ok(())
            }
        }
    }

    fn assign(&mut self, target: &Place, op: UpdateOp, value: &Expr) -> Result<()> {
        if target.path.is_empty() && op == UpdateOp::Set {
            self.expr(value)?;
            match self.resolve_var(&target.root) {
                Some(var) => self.store_var(var),
                None => {
                    let decl = self.declare(&target.root)?;
                    self.store_new(decl);
                }
            }
            return Ok(());
        }
        let Some(var) = self.resolve_var(&target.root) else {
            return Err(self.error(format!("`{}` is not defined", target.root), target.span));
        };
        let keys = self.path_keys(&target.path)?;
        self.expr(value)?;
        self.emit(Op::Update {
            var: VarRef::new(var),
            keys,
            op,
        });
        Ok(())
    }

    fn path_keys(&mut self, path: &[Key]) -> Result<u8> {
        let keys = u8::try_from(path.len()).map_err(|_| {
            Error::new(ErrorKind::Syntax, "path too long").at(self.source, self.span)
        })?;
        for key in path {
            match key {
                Key::Field(name) => {
                    let c = self.constant(Value::str(&**name))?;
                    self.emit(Op::Const(c));
                }
                Key::Index(index) => self.expr(index)?,
            }
        }
        Ok(keys)
    }

    /// Pushes a reference to `place`.
    fn make_ref(&mut self, place: &Place) -> Result<()> {
        let var = match self.resolve(&place.root) {
            Some(Resolved::Local(slot)) => Var::Cell(self.f().cell_for(slot)),
            Some(Resolved::Upval(u)) => Var::Upval(u),
            Some(Resolved::Global(g)) => Var::Global(g),
            Some(Resolved::Prelude(_)) => {
                return Err(self.error(
                    format!("can't take a reference to the builtin `{}`", place.root),
                    place.span,
                ));
            }
            None => return Err(self.error(format!("`{}` is not defined", place.root), place.span)),
        };
        let keys = self.path_keys(&place.path)?;
        self.span = place.span;
        self.emit(Op::MakeRef {
            var: VarRef::new(var),
            keys,
        });
        Ok(())
    }

    // ----- functions -----------------------------------------------------------------

    /// Compiles a function and pushes a closure for it.
    fn function(&mut self, decl: &FnDecl) -> Result<()> {
        let sig = Rc::new(decl.signature());
        self.funcs.push(FnState::new(
            decl.name.clone(),
            false,
            sig,
            decl.source.clone(),
        ));
        self.open_scope();
        for param in decl.params.iter().chain(decl.rest.iter()) {
            let slot = self.new_slot(&param.name)?;
            if param.by_ref {
                let cell = self.f().cell_for(slot);
                self.emit(Op::BindRef { slot, cell });
            } else {
                let at = self.emit(Op::Nop);
                self.f().prologue.push((slot, at));
            }
        }
        self.block_body(&decl.body.stmts)?;
        self.emit(Op::Return);
        let state = self.funcs.pop().unwrap_or_else(|| unreachable!());
        let proto = Rc::new(state.finish(self.source));
        let f = self.f();
        f.protos.push(proto);
        let index = (f.protos.len() - 1) as u32;
        self.span = decl.span;
        self.emit(Op::MakeClosure(index));
        Ok(())
    }

    // ----- expressions -----------------------------------------------------------------

    fn expr(&mut self, expr: &Expr) -> Result<()> {
        let outer = self.span;
        self.span = expr.span;
        let result = self.expr_inner(expr);
        self.span = outer;
        result
    }

    fn expr_inner(&mut self, expr: &Expr) -> Result<()> {
        match &expr.kind {
            ExprKind::Lit(value) => {
                match value {
                    Value::Null => self.emit(Op::Null),
                    Value::Bool(true) => self.emit(Op::True),
                    Value::Bool(false) => self.emit(Op::False),
                    other => {
                        let c = self.constant(other.clone())?;
                        self.emit(Op::Const(c))
                    }
                };
            }
            ExprKind::Var(name) => self.load(name, expr.span)?,
            ExprKind::Field(base, name) => {
                self.expr(base)?;
                let c = self.constant(Value::str(&**name))?;
                self.emit(Op::GetField(c));
            }
            ExprKind::Index(base, index) => {
                self.expr(base)?;
                self.expr(index)?;
                self.emit(Op::GetIndex);
            }
            ExprKind::Glue(parts) => {
                for part in parts {
                    self.expr(part)?;
                }
                self.emit(Op::Glue(parts.len() as u16));
            }
            ExprKind::Interp(parts) => {
                for part in parts {
                    self.expr(part)?;
                }
                self.emit(Op::Interp(parts.len() as u16));
            }
            ExprKind::Block(block) => self.block(block)?,
            ExprKind::Array(items) => self.array(items)?,
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    self.expr(key)?;
                    self.expr(value)?;
                }
                self.emit(Op::MakeMap(entries.len() as u32));
            }
            ExprKind::Unary(op, operand) => {
                self.expr(operand)?;
                self.emit(match op {
                    UnOp::Neg => Op::Neg,
                    UnOp::Not => Op::Not,
                });
            }
            ExprKind::Binary(op, lhs, rhs) => {
                self.expr(lhs)?;
                self.expr(rhs)?;
                self.emit(match op {
                    BinOp::Add => Op::Add,
                    BinOp::Sub => Op::Sub,
                    BinOp::Mul => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::Rem => Op::Rem,
                    BinOp::Eq => Op::Eq,
                    BinOp::Ne => Op::Ne,
                    BinOp::Lt => Op::Lt,
                    BinOp::Le => Op::Le,
                    BinOp::Gt => Op::Gt,
                    BinOp::Ge => Op::Ge,
                    BinOp::Range => Op::Range,
                });
            }
            ExprKind::Logic(op, lhs, rhs) => {
                self.expr(lhs)?;
                let jump = self.emit_jump(match op {
                    LogicOp::And => Op::JumpIfFalsyKeep,
                    LogicOp::Or => Op::JumpIfTruthyKeep,
                    LogicOp::Coalesce => Op::JumpIfNotNullKeep,
                });
                self.expr(rhs)?;
                self.patch_jump(jump);
            }
            ExprKind::Call(callee, args) => self.call(callee, args, expr.span)?,
            ExprKind::Lambda(decl) => self.function(decl)?,
            ExprKind::If(cond, then, otherwise) => {
                self.expr(cond)?;
                let to_else = self.emit_jump(Op::JumpIfFalse);
                self.expr(then)?;
                let to_end = self.emit_jump(Op::Jump);
                self.patch_jump(to_else);
                self.opt_expr(otherwise.as_deref())?;
                self.patch_jump(to_end);
            }
            ExprKind::While(cond, body) => {
                self.emit(Op::AccBegin);
                self.emit(Op::LoopEnter);
                self.f().loops.push(LoopCtx {
                    breaks: Vec::new(),
                    continues: Vec::new(),
                });
                let top = self.here();
                self.expr(cond)?;
                let exit = self.emit_jump(Op::JumpIfFalse);
                self.expr(body)?;
                self.emit(Op::AccPut);
                self.emit_loop(top);
                self.patch_jump(exit);
                self.end_loop(top);
                self.emit(Op::AccEnd);
            }
            ExprKind::For(for_loop) => self.for_loop(for_loop)?,
            ExprKind::Try(body, catch) => {
                let handler = self.emit_jump(Op::PushHandler);
                self.expr(body)?;
                self.emit(Op::PopHandler);
                let to_end = self.emit_jump(Op::Jump);
                self.patch_jump(handler);
                match catch {
                    Some((name, handler)) => {
                        self.open_scope();
                        let slot = self.new_slot(name)?;
                        self.store_new(Target::Slot(slot));
                        self.expr(handler)?;
                        self.close_scope();
                    }
                    None => {
                        self.emit(Op::Pop);
                        self.emit(Op::Null);
                    }
                }
                self.patch_jump(to_end);
            }
            ExprKind::Use(name) => {
                self.expr(name)?;
                self.emit(Op::Use);
            }
        }
        Ok(())
    }

    /// Ends the innermost loop: emits `LoopExit` and patches breaks and continues.
    fn end_loop(&mut self, continue_target: usize) {
        let ctx = self.f().loops.pop().unwrap_or_else(|| unreachable!());
        let exit = self.here();
        for at in ctx.breaks {
            self.patch_jump_to(at, exit);
        }
        for at in ctx.continues {
            self.patch_jump_to(at, continue_target);
        }
        self.emit(Op::LoopExit);
    }

    fn for_loop(&mut self, for_loop: &ForLoop) -> Result<()> {
        self.emit(Op::AccBegin);
        self.expr(&for_loop.iter)?;
        if let Some(split) = &for_loop.split {
            self.expr(split)?;
        }
        self.span = for_loop.iter.span;
        self.emit(Op::IterInit {
            split: for_loop.split.is_some(),
        });
        self.emit(Op::LoopEnter);
        self.f().loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        let top = self.here();
        let pairs = for_loop.second.is_some();
        let next = self.emit(Op::IterNext { pairs, exit: 0 });
        self.open_scope();
        if let Some(second) = &for_loop.second {
            let first = self.new_slot(&for_loop.first)?;
            let second = self.new_slot(second)?;
            self.store_new(Target::Slot(second));
            self.store_new(Target::Slot(first));
        } else {
            let first = self.new_slot(&for_loop.first)?;
            self.store_new(Target::Slot(first));
        }
        self.expr(&for_loop.body)?;
        self.emit(Op::AccPut);
        self.close_scope();
        self.emit_loop(top);
        self.patch_jump(next);
        self.end_loop(top);
        self.emit(Op::IterEnd);
        self.emit(Op::AccEnd);
        Ok(())
    }

    fn array(&mut self, items: &[Arg]) -> Result<()> {
        let plain = items
            .iter()
            .take_while(|a| matches!(a, Arg::Value(_)))
            .count();
        for item in &items[..plain] {
            if let Arg::Value(value) = item {
                self.expr(value)?;
            }
        }
        self.emit(Op::MakeArr(plain as u32));
        for item in &items[plain..] {
            match item {
                Arg::Value(value) => {
                    self.expr(value)?;
                    self.emit(Op::ArrPush);
                }
                Arg::Splat(value) => {
                    self.expr(value)?;
                    self.emit(Op::ArrExtend);
                }
                Arg::Ref(place) => {
                    return Err(Error::new(
                        ErrorKind::Syntax,
                        "references can't be stored in an array; they can only be passed to functions",
                    )
                    .at(self.source, place.span));
                }
            }
        }
        Ok(())
    }

    fn call(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<()> {
        if let ExprKind::Var(name) = &callee.kind {
            if self.resolve(name).is_none() {
                return Err(self.error(format!("unknown command `{name}`"), callee.span));
            }
        }
        self.expr(callee)?;
        if args.iter().any(|a| matches!(a, Arg::Splat(_))) {
            self.emit(Op::MakeArr(0));
            for arg in args {
                match arg {
                    Arg::Value(value) => {
                        self.expr(value)?;
                        self.emit(Op::ArrPush);
                    }
                    Arg::Splat(value) => {
                        self.expr(value)?;
                        self.emit(Op::ArrExtend);
                    }
                    Arg::Ref(place) => {
                        self.make_ref(place)?;
                        self.emit(Op::ArrPush);
                    }
                }
            }
            self.span = span;
            self.emit(Op::CallArr);
            return Ok(());
        }
        let count = u16::try_from(args.len()).map_err(|_| {
            Error::new(ErrorKind::Syntax, "too many arguments").at(self.source, span)
        })?;
        for arg in args {
            match arg {
                Arg::Value(value) => self.expr(value)?,
                Arg::Ref(place) => self.make_ref(place)?,
                Arg::Splat(_) => unreachable!(),
            }
        }
        self.span = span;
        self.emit(Op::Call(count));
        Ok(())
    }
}

impl FnDecl {
    /// The runtime signature of this declaration.
    pub fn signature(&self) -> Signature {
        let param = |p: &Param| SigParam {
            name: p.name.clone(),
            ty: p.ty,
            by_ref: p.by_ref,
        };
        Signature::new(
            self.params.iter().map(param).collect(),
            self.rest.as_ref().map(param),
        )
    }

    /// What makes two overloads indistinguishable.
    fn sig_key(&self) -> (Vec<(crate::value::Type, bool)>, bool) {
        (
            self.params.iter().map(|p| (p.ty, p.by_ref)).collect(),
            self.rest.is_some(),
        )
    }
}
