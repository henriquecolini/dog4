//! The bytecode interpreter.
//!
//! A stack machine with heap-allocated frames (deep recursion is bounded by
//! [`Limits::max_call_depth`], never by the Rust stack). Besides the operand
//! stack it keeps stacks of block accumulators, loop iterators, loop records
//! (for `break`/`continue`) and error handlers (for `try`).

mod binding;
mod call;
mod function;
mod iter;

use std::cell::RefCell;
use std::io::Write;
use std::rc::{Rc, Weak};

use rand::rngs::SmallRng;
use rustc_hash::FxHashMap;

pub use binding::{Binding, Cell, ModuleEnv, RefData, RefTarget};
pub use function::{
    Body, Function, NativeFn, Overload, ScriptFn, SigParam, Signature, parse_signature,
};

use crate::compile::{Capture, ModuleScope, Op, PreludeNames, Proto, Var, compile_module};
use crate::error::{Error, ErrorKind, Limit, Result};
use crate::limits::{Interrupt, Limits, Meter};
use crate::module::{ModuleProvider, ModuleSource, ProviderError};
use crate::syntax::{SourceMap, parse};
use crate::value::{
    Map, Type, UpdateOp, Value, add_assign, coerce, compare, div, get_key, key_text, mul, neg,
    range, rem, sub, update_path,
};
use iter::Iter;

/// How a frame's result is delivered when it returns.
enum FrameKind {
    /// A function call (or code run directly by the host): its value.
    Call,
    /// A module's top level: the module's map, cached under `id` if given.
    Module { id: Option<Rc<str>> },
}

struct Frame {
    func: Rc<ScriptFn>,
    ip: usize,
    /// Stack index of slot 0. The callee sits at `base - 1`.
    base: usize,
    cells: Vec<Binding>,
    acc_base: usize,
    iter_base: usize,
    loop_base: usize,
    handler_base: usize,
    kind: FrameKind,
}

#[derive(Clone, Copy)]
struct LoopRecord {
    stack_len: usize,
    acc_len: usize,
    iter_len: usize,
    handler_len: usize,
}

#[derive(Clone, Copy)]
struct Handler {
    frame_depth: usize,
    stack_len: usize,
    acc_len: usize,
    iter_len: usize,
    loop_len: usize,
    catch_ip: usize,
}

/// The interpreter state. Natives receive `&mut Vm` to call back into scripts,
/// print, and use the shared meter and random number generator.
pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    accs: Vec<Value>,
    iters: Vec<Iter>,
    loops: Vec<LoopRecord>,
    handlers: Vec<Handler>,
    native_depth: usize,
    pub(crate) meter: Meter,
    pub(crate) limits: Limits,
    pub(crate) interrupt: Interrupt,
    pub(crate) prelude: Vec<Value>,
    pub(crate) prelude_names: PreludeNames,
    pub(crate) sources: SourceMap,
    pub(crate) provider: Option<Box<dyn ModuleProvider>>,
    pub(crate) native_modules: FxHashMap<Rc<str>, Value>,
    cache: FxHashMap<Rc<str>, Value>,
    loading: Vec<Rc<str>>,
    pub(crate) out: Box<dyn Write>,
    pub(crate) rng: SmallRng,
    cells: Vec<Weak<RefCell<Value>>>,
    sweep_at: usize,
    /// Spare argument buffers for native calls, so calls don't allocate.
    pub(crate) arg_pool: Vec<Vec<Value>>,
    envs: Vec<Weak<ModuleEnv>>,
    placeholder: Cell,
}

fn jump(ip: usize, offset: i32) -> usize {
    (ip as isize + offset as isize) as usize
}

impl Vm {
    pub(crate) fn new(
        limits: Limits,
        prelude: Vec<Value>,
        prelude_names: PreludeNames,
        out: Box<dyn Write>,
        rng: SmallRng,
    ) -> Vm {
        let interrupt = Interrupt::default();
        Vm {
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(16),
            accs: Vec::with_capacity(16),
            iters: Vec::new(),
            loops: Vec::new(),
            handlers: Vec::new(),
            native_depth: 0,
            meter: Meter::new(&limits, interrupt.clone()),
            limits,
            interrupt,
            prelude,
            prelude_names,
            sources: SourceMap::default(),
            provider: None,
            native_modules: FxHashMap::default(),
            cache: FxHashMap::default(),
            loading: Vec::new(),
            out,
            rng,
            cells: Vec::new(),
            sweep_at: 1024,
            arg_pool: Vec::new(),
            envs: Vec::new(),
            placeholder: Rc::new(RefCell::new(Value::Null)),
        }
    }

    // ----- public helpers for natives -----------------------------------------------------

    /// Calls a dog4 function with arguments and returns its result.
    pub fn call(&mut self, function: &Value, args: Vec<Value>) -> Result<Value> {
        if self.native_depth >= self.limits.max_native_depth {
            return Err(Error::limit(Limit::NativeDepth));
        }
        let stop = self.frames.len();
        let stack_len = self.stack.len();
        let argc = args.len();
        self.stack.push(function.clone());
        self.stack.extend(args);
        self.native_depth += 1;
        let result = match self.call_value(argc) {
            Ok(true) => self.run(stop),
            Ok(false) => Ok(self.pop()),
            Err(e) => Err(e),
        };
        self.native_depth -= 1;
        if result.is_err() {
            self.stack.truncate(stack_len);
        }
        result
    }

    /// Writes text to the runtime's output (counted against the output limit).
    pub fn print(&mut self, text: &str) -> Result<()> {
        self.meter.output(text.len())?;
        self.out
            .write_all(text.as_bytes())
            .map_err(|e| Error::new(ErrorKind::Io, format!("can't print: {e}")))
    }

    /// The resource meter, for natives that do proportional work.
    pub fn meter(&mut self) -> &mut Meter {
        &mut self.meter
    }

    /// The random number generator.
    pub fn rng(&mut self) -> &mut SmallRng {
        &mut self.rng
    }

    /// Looks a name up the way a script in the current module would see it
    /// (module globals, then the prelude).
    pub fn lookup(&self, name: &str) -> Option<Value> {
        if let Some(frame) = self.frames.last() {
            if let Some(value) = frame.func.module.lookup(name) {
                return Some(value);
            }
        }
        self.prelude_names
            .get(name)
            .map(|i| self.prelude[i as usize].clone())
    }

    /// Compiles and runs `code` inside the current module, so it can see (and
    /// define) the module's globals. Returns what the code outputs.
    pub fn eval(&mut self, code: &str) -> Result<Value> {
        let env = match self.frames.last() {
            Some(frame) => frame.func.module.clone(),
            None => self.new_env("<eval>".into(), ModuleScope::default()),
        };
        self.eval_in(&env, "<eval>", code)
    }

    pub(crate) fn eval_in(&mut self, env: &Rc<ModuleEnv>, name: &str, code: &str) -> Result<Value> {
        let source = self.sources.add(name, code);
        let block = parse(code, source, self.limits.max_nesting)?;
        let proto = {
            let mut scope = env.scope.borrow_mut();
            compile_module(&block, &self.prelude_names, &mut scope, source)?
        };
        env.sync();
        self.run_proto(proto, env.clone(), FrameKind::Call)
    }

    // ----- running ---------------------------------------------------------------------

    pub(crate) fn new_env(&mut self, name: Rc<str>, scope: ModuleScope) -> Rc<ModuleEnv> {
        let env = Rc::new(ModuleEnv::new(name, scope));
        self.envs.push(Rc::downgrade(&env));
        env
    }

    /// Parses and compiles a source as a new module.
    pub(crate) fn compile_source(
        &mut self,
        name: &Rc<str>,
        text: &str,
    ) -> Result<(Rc<Proto>, Rc<ModuleEnv>)> {
        let source = self.sources.add(name.clone(), text);
        let block = parse(text, source, self.limits.max_nesting)?;
        let mut scope = ModuleScope::default();
        let proto = compile_module(&block, &self.prelude_names, &mut scope, source)?;
        let env = self.new_env(name.clone(), scope);
        Ok((proto, env))
    }

    /// Runs a source as an entry script and returns what it outputs.
    pub(crate) fn run_source(&mut self, name: &str, text: &str) -> Result<Value> {
        let (proto, env) = self.compile_source(&name.into(), text)?;
        self.run_proto(proto, env, FrameKind::Call)
    }

    /// Loads a module (running it if needed) and returns its map.
    pub(crate) fn import(&mut self, name: &str) -> Result<Value> {
        let stop = self.frames.len();
        match self.use_module(Value::str(name))? {
            Some(map) => Ok(map),
            None => self.run(stop),
        }
    }

    fn run_proto(
        &mut self,
        proto: Rc<Proto>,
        env: Rc<ModuleEnv>,
        kind: FrameKind,
    ) -> Result<Value> {
        if self.native_depth >= self.limits.max_native_depth {
            return Err(Error::limit(Limit::NativeDepth));
        }
        let stop = self.frames.len();
        self.native_depth += 1;
        let result = self
            .enter_module(proto, env, kind)
            .and_then(|_| self.run(stop));
        self.native_depth -= 1;
        result
    }

    fn enter_module(
        &mut self,
        proto: Rc<Proto>,
        env: Rc<ModuleEnv>,
        kind: FrameKind,
    ) -> Result<()> {
        let func = Rc::new(ScriptFn {
            proto,
            upvals: Box::new([]),
            module: env,
        });
        self.stack.push(Value::Null);
        let base = self.stack.len();
        self.push_frame(func, base, kind)
    }

    /// Resets the meter for a new top-level run.
    pub(crate) fn reset_meter(&mut self) {
        self.meter = Meter::new(&self.limits, self.interrupt.clone());
    }

    fn push_frame(&mut self, func: Rc<ScriptFn>, base: usize, kind: FrameKind) -> Result<()> {
        if self.frames.len() >= self.limits.max_call_depth {
            return Err(Error::limit(Limit::CallDepth));
        }
        let needed = base + func.proto.n_slots as usize;
        if self.stack.len() < needed {
            self.stack.resize(needed, Value::Null);
        }
        let cells = match func.proto.n_cells {
            0 => Vec::new(),
            n => vec![Binding::Cell(self.placeholder.clone()); n as usize],
        };
        self.frames.push(Frame {
            func,
            ip: 0,
            base,
            cells,
            acc_base: self.accs.len(),
            iter_base: self.iters.len(),
            loop_base: self.loops.len(),
            handler_base: self.handlers.len(),
            kind,
        });
        Ok(())
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().unwrap_or_default()
    }

    fn new_cell(&mut self, value: Value) -> Cell {
        let cell = Rc::new(RefCell::new(value));
        if self.cells.len() >= self.sweep_at {
            self.cells.retain(|w| w.strong_count() > 0);
            self.sweep_at = (self.cells.len() * 2).max(1024);
        }
        self.cells.push(Rc::downgrade(&cell));
        cell
    }

    /// Runs until the frame at depth `stop` returns, handling errors with `try`
    /// handlers that belong to this run.
    pub(crate) fn run(&mut self, stop: usize) -> Result<Value> {
        loop {
            match self.dispatch(stop) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let err = self.locate(err);
                    if err.kind().is_catchable() {
                        if let Some(handler) = self.handlers.last().copied() {
                            if handler.frame_depth > stop {
                                self.catch(handler, err);
                                continue;
                            }
                        }
                    }
                    let err = self.trace(err, stop);
                    self.unwind_to(stop);
                    return Err(err);
                }
            }
        }
    }

    fn locate(&self, err: Error) -> Error {
        if err.loc().is_some() {
            return err;
        }
        match self.frames.last() {
            Some(frame) => {
                let proto = &frame.func.proto;
                let at = frame
                    .ip
                    .saturating_sub(1)
                    .min(proto.spans.len().saturating_sub(1));
                match proto.spans.get(at) {
                    Some(&span) => err.at(proto.source, span),
                    None => err,
                }
            }
            None => err,
        }
    }

    fn trace(&self, mut err: Error, stop: usize) -> Error {
        for depth in (stop.max(1)..self.frames.len()).rev() {
            let callee = &self.frames[depth].func.proto;
            if matches!(self.frames[depth].kind, FrameKind::Module { .. }) {
                continue;
            }
            let caller = &self.frames[depth - 1];
            let proto = &caller.func.proto;
            let at = caller.ip.saturating_sub(1);
            let loc = proto.spans.get(at).map(|&span| (proto.source, span));
            err.push_trace(callee.name.clone(), loc);
        }
        err
    }

    fn catch(&mut self, handler: Handler, err: Error) {
        self.handlers.pop();
        while self.frames.len() > handler.frame_depth {
            self.discard_frame();
        }
        self.stack.truncate(handler.stack_len);
        self.accs.truncate(handler.acc_len);
        self.iters.truncate(handler.iter_len);
        self.loops.truncate(handler.loop_len);
        self.stack.push(err.to_value());
        if let Some(frame) = self.frames.last_mut() {
            frame.ip = handler.catch_ip;
        }
    }

    fn discard_frame(&mut self) {
        if let Some(frame) = self.frames.pop() {
            if let FrameKind::Module { id: Some(_) } = frame.kind {
                self.loading.pop();
            }
        }
    }

    fn unwind_to(&mut self, stop: usize) {
        let Some(bottom) = self.frames.get(stop) else {
            return;
        };
        let (base, acc, iter, lp, handler) = (
            bottom.base,
            bottom.acc_base,
            bottom.iter_base,
            bottom.loop_base,
            bottom.handler_base,
        );
        while self.frames.len() > stop {
            self.discard_frame();
        }
        self.stack.truncate(base.saturating_sub(1));
        self.accs.truncate(acc);
        self.iters.truncate(iter);
        self.loops.truncate(lp);
        self.handlers.truncate(handler);
    }

    fn unwind_loop(&mut self) -> Result<()> {
        let Some(record) = self.loops.last().copied() else {
            return Err(Error::new(
                ErrorKind::Value,
                "internal error: no loop to leave",
            ));
        };
        self.stack.truncate(record.stack_len);
        self.iters.truncate(record.iter_len);
        self.handlers.truncate(record.handler_len);
        if self.accs.len() > record.acc_len {
            self.accs.truncate(record.acc_len + 1);
            let partial = self.accs.pop().unwrap_or_default();
            if let Some(acc) = self.accs.last_mut() {
                add_assign(acc, partial, &mut self.meter)?;
            }
        }
        Ok(())
    }

    /// Runs `use name`: returns the map directly when it's cached or native, or
    /// pushes the module's frame (returning `None`) when it must run first.
    fn use_module(&mut self, name: Value) -> Result<Option<Value>> {
        let Value::Str(name) = name else {
            return Err(Error::type_error(format!(
                "a module name must be a string, not {}",
                name.describe()
            )));
        };
        if let Some(module) = self.native_modules.get(name.as_str()) {
            return Ok(Some(module.clone()));
        }
        let failed = |e: ProviderError| match e {
            ProviderError::NotFound => {
                Error::new(ErrorKind::Module, format!("module `{name}` not found"))
            }
            ProviderError::Failed(message) => Error::new(
                ErrorKind::Module,
                format!("can't load module `{name}`: {message}"),
            ),
        };
        let Some(provider) = &self.provider else {
            return Err(Error::new(
                ErrorKind::Module,
                format!("can't load module `{name}`: no module provider is configured"),
            ));
        };
        let from = self.frames.last().map(|f| f.func.module.name.clone());
        let id = provider.resolve(&name, from.as_deref()).map_err(failed)?;
        if let Some(module) = self.cache.get(id.as_str()) {
            return Ok(Some(module.clone()));
        }
        if let Some(start) = self.loading.iter().position(|l| **l == *id) {
            let mut cycle: Vec<&str> = self.loading[start..].iter().map(|s| &**s).collect();
            cycle.push(&id);
            return Err(Error::new(
                ErrorKind::Module,
                format!("circular import: {}", cycle.join(" → ")),
            ));
        }
        let source = provider.load(&id).map_err(failed)?;
        let id: Rc<str> = id.into();
        match source {
            ModuleSource::Native(module) => {
                self.cache.insert(id, module.clone());
                Ok(Some(module))
            }
            ModuleSource::Script(text) => {
                let (proto, env) = self.compile_source(&id, &text)?;
                self.enter_module(
                    proto,
                    env,
                    FrameKind::Module {
                        id: Some(id.clone()),
                    },
                )?;
                self.loading.push(id);
                Ok(None)
            }
        }
    }

    // ----- the dispatch loop ---------------------------------------------------------------

    fn dispatch(&mut self, stop: usize) -> Result<Value> {
        let mut fi = self.frames.len() - 1;
        let mut func = self.frames[fi].func.clone();
        let mut proto = func.proto.clone();
        let mut ip = self.frames[fi].ip;
        let mut base = self.frames[fi].base;

        macro_rules! fail {
            ($err:expr) => {{
                self.frames[fi].ip = ip;
                return Err($err);
            }};
        }
        macro_rules! t {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => fail!(e),
                }
            };
        }
        macro_rules! reload {
            () => {{
                fi = self.frames.len() - 1;
                func = self.frames[fi].func.clone();
                proto = func.proto.clone();
                ip = self.frames[fi].ip;
                base = self.frames[fi].base;
            }};
        }
        macro_rules! binary {
            ($f:expr) => {{
                let b = self.pop();
                let a = self.pop();
                let r = t!($f(&a, &b));
                a.discard();
                b.discard();
                self.stack.push(r);
            }};
        }
        // Integer arithmetic in place on the top of the stack, falling back to
        // the general operation for anything else (or on overflow).
        macro_rules! arith {
            ($checked:ident, $general:expr) => {{
                let b = self.pop();
                if let (Some(Value::Int(x)), Value::Int(y)) = (self.stack.last_mut(), &b) {
                    if let Some(r) = x.$checked(*y) {
                        *x = r;
                        b.discard();
                        continue;
                    }
                }
                let a = self.pop();
                let r = t!($general(&a, &b));
                a.discard();
                b.discard();
                self.stack.push(r);
            }};
        }
        macro_rules! compare_op {
            ($test:expr) => {{
                let b = self.pop();
                let a = self.pop();
                let result = match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => $test(x.cmp(y)),
                    _ => t!(compare(&a, &b)).is_some_and($test),
                };
                a.discard();
                b.discard();
                self.stack.push(Value::Bool(result));
            }};
        }

        loop {
            let op = proto.code[ip];
            ip += 1;
            match op {
                Op::Nop => {}
                Op::Const(c) => self.stack.push(proto.consts[c as usize].clone()),
                Op::Null => self.stack.push(Value::Null),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Pop => self.pop().discard(),

                Op::LoadLocal(s) => {
                    let v = self.stack[base + s as usize].clone();
                    self.stack.push(v);
                }
                Op::StoreLocal(s) => {
                    let v = self.pop();
                    std::mem::replace(&mut self.stack[base + s as usize], v).discard();
                }
                Op::LoadCell(c) => {
                    let v = t!(self.frames[fi].cells[c as usize].read());
                    self.stack.push(v);
                }
                Op::StoreCell(c) => {
                    let v = self.pop();
                    t!(self.frames[fi].cells[c as usize].write(v, &mut self.meter));
                }
                Op::DeclCell(c) => {
                    let v = self.pop();
                    let cell = self.new_cell(v);
                    self.frames[fi].cells[c as usize] = Binding::Cell(cell);
                }
                Op::NewCell(c) => {
                    let cell = self.new_cell(Value::Null);
                    self.frames[fi].cells[c as usize] = Binding::Cell(cell);
                }
                Op::BoxParam { slot, cell } => {
                    let v = std::mem::take(&mut self.stack[base + slot as usize]);
                    let cell_rc = self.new_cell(v);
                    self.frames[fi].cells[cell as usize] = Binding::Cell(cell_rc);
                }
                Op::BindRef { slot, cell } => {
                    match std::mem::take(&mut self.stack[base + slot as usize]) {
                        Value::Ref(r) => self.frames[fi].cells[cell as usize] = Binding::Ref(r),
                        other => fail!(Error::type_error(format!(
                            "expected a reference, got {}",
                            other.describe()
                        ))),
                    }
                }
                Op::DeclRef(c) => match self.pop() {
                    Value::Ref(r) => self.frames[fi].cells[c as usize] = Binding::Ref(r),
                    other => fail!(Error::type_error(format!(
                        "expected a reference, got {}",
                        other.describe()
                    ))),
                },
                Op::LoadUpval(u) => {
                    let v = t!(func.upvals[u as usize].read());
                    self.stack.push(v);
                }
                Op::StoreUpval(u) => {
                    let v = self.pop();
                    t!(func.upvals[u as usize].write(v, &mut self.meter));
                }
                Op::LoadGlobal(g) => {
                    let v = t!(func.module.get(g));
                    self.stack.push(v);
                }
                Op::StoreGlobal(g) => {
                    let v = self.pop();
                    func.module.set(g, v);
                }
                Op::LoadPrelude(p) => self.stack.push(self.prelude[p as usize].clone()),
                Op::Update { var, keys, op } => {
                    let value = self.pop();
                    if keys == 0 && op == UpdateOp::Add {
                        if let Var::Local(s) = var.get() {
                            if let (Value::Int(x), Value::Int(y)) =
                                (&mut self.stack[base + s as usize], &value)
                            {
                                if let Some(r) = x.checked_add(*y) {
                                    *x = r;
                                    value.discard();
                                    continue;
                                }
                            }
                        }
                    }
                    let path: Vec<Value> = match keys {
                        0 => Vec::new(),
                        n => {
                            let start = self.stack.len() - n as usize;
                            self.stack.drain(start..).collect()
                        }
                    };
                    let result = match var.get() {
                        Var::Local(s) => update_path(
                            &mut self.stack[base + s as usize],
                            &path,
                            op,
                            value,
                            &mut self.meter,
                        ),
                        Var::Cell(c) => self.frames[fi].cells[c as usize].update(
                            &path,
                            op,
                            value,
                            &mut self.meter,
                        ),
                        Var::Upval(u) => {
                            func.upvals[u as usize].update(&path, op, value, &mut self.meter)
                        }
                        Var::Global(g) => func.module.update(g, &path, op, value, &mut self.meter),
                    };
                    t!(result);
                }
                Op::MakeRef { var, keys } => {
                    let start = self.stack.len() - keys as usize;
                    let path: Vec<Value> = self.stack.drain(start..).collect();
                    let data = match var.get() {
                        Var::Cell(c) => self.frames[fi].cells[c as usize].make_ref(path),
                        Var::Upval(u) => func.upvals[u as usize].make_ref(path),
                        Var::Global(g) => {
                            RefData::new(RefTarget::Global(func.module.clone(), g), path)
                        }
                        Var::Local(_) => fail!(Error::new(
                            ErrorKind::Value,
                            "internal error: reference to an unboxed local"
                        )),
                    };
                    self.stack.push(Value::Ref(Rc::new(data)));
                }

                Op::GetField(c) => {
                    let v = self.pop();
                    let r = t!(get_key(&v, &proto.consts[c as usize]));
                    self.stack.push(r);
                }
                Op::GetIndex => {
                    let key = self.pop();
                    let v = self.pop();
                    let r = t!(get_key(&v, &key));
                    self.stack.push(r);
                }

                Op::Neg => {
                    let v = self.pop();
                    let r = t!(neg(&v));
                    self.stack.push(r);
                }
                Op::Not => {
                    let v = self.pop();
                    self.stack.push(Value::Bool(!v.truthy()));
                }
                Op::Add => {
                    let b = self.pop();
                    let Some(a) = self.stack.last_mut() else {
                        fail!(Error::new(ErrorKind::Value, "internal error: empty stack"));
                    };
                    if let (Value::Int(x), Value::Int(y)) = (&mut *a, &b) {
                        if let Some(r) = x.checked_add(*y) {
                            *x = r;
                            b.discard();
                            continue;
                        }
                    }
                    t!(add_assign(a, b, &mut self.meter));
                }
                Op::Sub => arith!(checked_sub, sub),
                Op::Mul => {
                    let b = self.pop();
                    if let (Some(Value::Int(x)), Value::Int(y)) = (self.stack.last_mut(), &b) {
                        if let Some(r) = x.checked_mul(*y) {
                            *x = r;
                            b.discard();
                            continue;
                        }
                    }
                    let a = self.pop();
                    let r = t!(mul(&a, &b, &mut self.meter));
                    self.stack.push(r);
                }
                Op::Div => binary!(div),
                Op::Rem => binary!(rem),
                Op::Eq => {
                    let b = self.pop();
                    let a = self.pop();
                    self.stack.push(Value::Bool(a == b));
                }
                Op::Ne => {
                    let b = self.pop();
                    let a = self.pop();
                    self.stack.push(Value::Bool(a != b));
                }
                Op::Lt => compare_op!(std::cmp::Ordering::is_lt),
                Op::Le => compare_op!(std::cmp::Ordering::is_le),
                Op::Gt => compare_op!(std::cmp::Ordering::is_gt),
                Op::Ge => compare_op!(std::cmp::Ordering::is_ge),
                Op::Range => {
                    let b = self.pop();
                    let a = self.pop();
                    let r = t!(range(&a, &b, &mut self.meter));
                    self.stack.push(r);
                }
                Op::Glue(n) => {
                    let r = t!(glue(&mut self.stack, n as usize, &mut self.meter));
                    self.stack.push(r);
                }
                Op::Interp(n) => {
                    let r = t!(interpolate(&mut self.stack, n as usize, &mut self.meter));
                    self.stack.push(r);
                }

                Op::MakeArr(n) => {
                    t!(self.meter.coll_len(n as usize));
                    let start = self.stack.len() - n as usize;
                    let items: Vec<Value> = self.stack.drain(start..).collect();
                    self.stack.push(Value::arr(items));
                }
                Op::ArrPush => {
                    let v = self.pop();
                    if let Some(Value::Arr(items)) = self.stack.last_mut() {
                        t!(self.meter.grow_coll(items.len() + 1, 1));
                        items.make_mut().push(v);
                    }
                }
                Op::ArrExtend => {
                    let v = self.pop();
                    let more = match v {
                        Value::Arr(more) => more,
                        other => match coerce(other, Type::ARR) {
                            Ok(Value::Arr(more)) => more,
                            Ok(_) | Err(_) => fail!(Error::type_error(
                                "only arrays (and text) can be spread with `$%`"
                            )),
                        },
                    };
                    if let Some(Value::Arr(items)) = self.stack.last_mut() {
                        t!(self.meter.grow_coll(items.len() + more.len(), more.len()));
                        items.make_mut().extend(more.into_vec());
                    }
                }
                Op::MakeMap(n) => {
                    t!(self.meter.coll_len(n as usize));
                    let start = self.stack.len() - 2 * n as usize;
                    let mut map = Map::default();
                    let mut pairs = self.stack.drain(start..);
                    while let (Some(k), Some(v)) = (pairs.next(), pairs.next()) {
                        map.insert(key_text(&k), v);
                    }
                    drop(pairs);
                    self.stack.push(Value::map(map));
                }

                Op::Jump(o) => {
                    if o < 0 {
                        t!(self.meter.tick(1));
                    }
                    ip = jump(ip, o);
                }
                Op::JumpIfFalse(o) => {
                    let v = self.pop();
                    let truthy = v.truthy();
                    v.discard();
                    if !truthy {
                        ip = jump(ip, o);
                    }
                }
                Op::JumpIfFalsyKeep(o) => {
                    if self.stack.last().is_some_and(|v| !v.truthy()) {
                        ip = jump(ip, o);
                    } else {
                        self.stack.pop();
                    }
                }
                Op::JumpIfTruthyKeep(o) => {
                    if self.stack.last().is_some_and(Value::truthy) {
                        ip = jump(ip, o);
                    } else {
                        self.stack.pop();
                    }
                }
                Op::JumpIfNotNullKeep(o) => {
                    if self.stack.last().is_some_and(|v| !v.is_null()) {
                        ip = jump(ip, o);
                    } else {
                        self.stack.pop();
                    }
                }

                Op::Call(n) => {
                    self.frames[fi].ip = ip;
                    if t!(self.call_value(n as usize)) {
                        reload!();
                    }
                }
                Op::CallArr => {
                    let args = self.pop();
                    let Value::Arr(args) = args else {
                        fail!(Error::new(
                            ErrorKind::Value,
                            "internal error: CallArr without an array"
                        ));
                    };
                    let n = args.len();
                    self.stack.extend(args.into_vec());
                    self.frames[fi].ip = ip;
                    if t!(self.call_value(n)) {
                        reload!();
                    }
                }
                Op::Return => {
                    let value = self.pop();
                    let frame = self.frames.pop().unwrap_or_else(|| unreachable!());
                    self.stack.truncate(frame.base.saturating_sub(1));
                    self.accs.truncate(frame.acc_base);
                    self.iters.truncate(frame.iter_base);
                    self.loops.truncate(frame.loop_base);
                    self.handlers.truncate(frame.handler_base);
                    let value = match frame.kind {
                        FrameKind::Call => value,
                        FrameKind::Module { id } => {
                            let exports = func.module.exports();
                            if let Some(id) = id {
                                self.loading.pop();
                                self.cache.insert(id, exports.clone());
                            }
                            exports
                        }
                    };
                    if self.frames.len() == stop {
                        return Ok(value);
                    }
                    self.stack.push(value);
                    reload!();
                }

                Op::AccBegin => self.accs.push(Value::Null),
                Op::AccPut => {
                    let v = self.pop();
                    if v.is_null() {
                        v.discard();
                    } else if let Some(acc) = self.accs.last_mut() {
                        if acc.is_null() {
                            std::mem::replace(acc, v).discard();
                        } else {
                            t!(add_assign(acc, v, &mut self.meter));
                        }
                    }
                }
                Op::AccEnd => {
                    let v = self.accs.pop().unwrap_or_default();
                    self.stack.push(v);
                }
                Op::AccClear => {
                    let v = self.pop();
                    if let Some(acc) = self.accs.last_mut() {
                        *acc = v;
                    }
                }

                Op::MakeClosure(p) => {
                    let child = proto.protos[p as usize].clone();
                    let frame = &self.frames[fi];
                    let upvals: Box<[Binding]> = child
                        .captures
                        .iter()
                        .map(|capture| match *capture {
                            Capture::Cell(c) => frame.cells[c as usize].clone(),
                            Capture::Upval(u) => func.upvals[u as usize].clone(),
                        })
                        .collect();
                    let script = Rc::new(ScriptFn {
                        proto: child.clone(),
                        upvals,
                        module: func.module.clone(),
                    });
                    let function = Function {
                        name: child.name.clone(),
                        overloads: vec![Overload {
                            sig: child.sig.clone(),
                            body: Body::Script(script),
                        }],
                    };
                    self.stack.push(Value::Func(Rc::new(function)));
                }
                Op::MakeOverloads(n) => {
                    let start = self.stack.len() - n as usize;
                    let mut name: Option<Rc<str>> = None;
                    let mut overloads = Vec::new();
                    for v in self.stack.drain(start..) {
                        if let Value::Func(f) = v {
                            name.get_or_insert_with(|| f.name.clone());
                            overloads.extend(f.overloads.iter().cloned());
                        }
                    }
                    let function = Function {
                        name: name.unwrap_or_else(|| "<fn>".into()),
                        overloads,
                    };
                    self.stack.push(Value::Func(Rc::new(function)));
                }

                Op::IterInit { split } => {
                    let sep = if split { Some(self.pop()) } else { None };
                    let v = self.pop();
                    let it = t!(Iter::new(v, sep, &mut self.meter));
                    self.iters.push(it);
                }
                Op::IterRange => {
                    let hi = self.pop();
                    let lo = self.pop();
                    let it = t!(Iter::range(&lo, &hi));
                    self.iters.push(it);
                }
                Op::IterNext { pairs, exit } => {
                    match self.iters.last_mut().and_then(|it| it.next(pairs)) {
                        Some((first, second)) => {
                            self.stack.push(first);
                            if let Some(second) = second {
                                self.stack.push(second);
                            }
                        }
                        None => ip = jump(ip, exit),
                    }
                }
                Op::IterEnd => {
                    self.iters.pop();
                }
                Op::LoopEnter => self.loops.push(LoopRecord {
                    stack_len: self.stack.len(),
                    acc_len: self.accs.len(),
                    iter_len: self.iters.len(),
                    handler_len: self.handlers.len(),
                }),
                Op::LoopExit => {
                    self.loops.pop();
                }
                Op::Break(o) => {
                    let v = self.pop();
                    t!(self.unwind_loop());
                    if let Some(acc) = self.accs.last_mut() {
                        t!(add_assign(acc, v, &mut self.meter));
                    }
                    ip = jump(ip, o);
                }
                Op::Continue(o) => {
                    t!(self.unwind_loop());
                    ip = jump(ip, o);
                }

                Op::PushHandler(o) => self.handlers.push(Handler {
                    frame_depth: self.frames.len(),
                    stack_len: self.stack.len(),
                    acc_len: self.accs.len(),
                    iter_len: self.iters.len(),
                    loop_len: self.loops.len(),
                    catch_ip: jump(ip, o),
                }),
                Op::PopHandler => {
                    self.handlers.pop();
                }
                Op::Throw => {
                    let v = self.pop();
                    fail!(Error::thrown(v));
                }

                Op::Use => {
                    let name = self.pop();
                    self.frames[fi].ip = ip;
                    match t!(self.use_module(name)) {
                        Some(module) => self.stack.push(module),
                        None => reload!(),
                    }
                }
                Op::MakeModule(m) => {
                    let module = proto.modules[m as usize].clone();
                    let mut scope = ModuleScope::default();
                    for name in &module.globals {
                        scope.global(name);
                    }
                    let env = self.new_env(func.module.name.clone(), scope);
                    self.frames[fi].ip = ip;
                    t!(self.enter_module(module.main.clone(), env, FrameKind::Module { id: None }));
                    reload!();
                }
            }
        }
    }
}

/// Folds the top `n` values left to right (a glued word).
fn glue(stack: &mut Vec<Value>, n: usize, meter: &mut Meter) -> Result<Value> {
    let start = stack.len() - n;
    let mut parts = stack.drain(start..);
    let mut acc = parts.next().unwrap_or_default();
    for part in parts {
        add_assign(&mut acc, part, meter)?;
    }
    Ok(acc)
}

/// Concatenates the text of the top `n` values (a `"…"` string).
fn interpolate(stack: &mut Vec<Value>, n: usize, meter: &mut Meter) -> Result<Value> {
    use std::fmt::Write as _;
    let start = stack.len() - n;
    let mut text = String::new();
    for part in stack.drain(start..) {
        match &part {
            Value::Str(s) => text.push_str(s),
            other => {
                let _ = write!(text, "{other}");
            }
        }
    }
    meter.str_len(text.len())?;
    Ok(Value::str(text))
}

impl Drop for Vm {
    fn drop(&mut self) {
        // Closures can form reference cycles through cells and module globals.
        // Break them all so a dropped runtime frees everything it allocated.
        self.stack.clear();
        self.frames.clear();
        self.accs.clear();
        self.iters.clear();
        self.cache.clear();
        for cell in std::mem::take(&mut self.cells) {
            if let Some(cell) = cell.upgrade() {
                let old = std::mem::take(&mut *cell.borrow_mut());
                drop(old);
            }
        }
        for env in std::mem::take(&mut self.envs) {
            if let Some(env) = env.upgrade() {
                env.clear();
            }
        }
    }
}
