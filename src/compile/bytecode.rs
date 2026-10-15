//! Bytecode for the stack VM.
//!
//! Every function body (and every module's top level) compiles to a [`Proto`]:
//! a flat array of [`Op`]s plus its constants. Variables are resolved at compile
//! time into numbered slots, captured cells, upvalues or module globals.
//!
//! Blocks fold their statements' values into an *accumulator*: `AccBegin` pushes
//! a fresh one (`null`), `AccPut` folds the top of the operand stack into it, and
//! `AccEnd` pops it back onto the operand stack as the block's value.

use std::rc::Rc;

use super::compiler::ModuleScope;
use crate::syntax::{SourceId, Span};
use crate::value::{UpdateOp, Value};
use crate::vm::Signature;

/// Where a variable lives. Packed into 32 bits so [`Op`] stays 8 bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VarRef(u32);

/// The unpacked form of a [`VarRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Var {
    /// A plain frame slot.
    Local(u16),
    /// A frame cell (a local captured by a closure, or a reference binding).
    Cell(u16),
    /// A captured variable of the enclosing function.
    Upval(u16),
    /// A module global.
    Global(u32),
}

impl VarRef {
    const SHIFT: u32 = 30;
    const MASK: u32 = (1 << Self::SHIFT) - 1;

    /// Packs a [`Var`].
    pub fn new(var: Var) -> VarRef {
        let (tag, index) = match var {
            Var::Local(i) => (0, i as u32),
            Var::Cell(i) => (1, i as u32),
            Var::Upval(i) => (2, i as u32),
            Var::Global(i) => (3, i & Self::MASK),
        };
        VarRef(tag << Self::SHIFT | index)
    }

    /// Unpacks into a [`Var`].
    pub fn get(self) -> Var {
        let index = self.0 & Self::MASK;
        match self.0 >> Self::SHIFT {
            0 => Var::Local(index as u16),
            1 => Var::Cell(index as u16),
            2 => Var::Upval(index as u16),
            _ => Var::Global(index),
        }
    }
}

impl std::fmt::Debug for VarRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

/// One VM instruction. Jump offsets are relative to the next instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Does nothing (a patchable placeholder).
    Nop,
    /// Pushes `consts[i]`.
    Const(u32),
    /// Pushes `null`.
    Null,
    /// Pushes `true`.
    True,
    /// Pushes `false`.
    False,
    /// Discards the top value.
    Pop,

    /// Pushes a plain local.
    LoadLocal(u16),
    /// Pops into a plain local.
    StoreLocal(u16),
    /// Pushes the value of a cell.
    LoadCell(u16),
    /// Pops into a cell.
    StoreCell(u16),
    /// Pops into a *fresh* cell (each loop iteration gets its own variable).
    DeclCell(u16),
    /// Makes a fresh empty cell (hoisted functions that capture each other).
    NewCell(u16),
    /// Moves parameter `slot` into fresh cell `cell` (the parameter is captured).
    BoxParam {
        /// The parameter slot.
        slot: u16,
        /// The cell to create.
        cell: u16,
    },
    /// Binds reference parameter `slot` (a `Value::Ref`) as cell `cell`.
    BindRef {
        /// The parameter slot.
        slot: u16,
        /// The cell to bind.
        cell: u16,
    },
    /// Pops a reference and binds it as cell `cell` (`let r = &x`).
    DeclRef(u16),
    /// Pushes an upvalue.
    LoadUpval(u16),
    /// Pops into an upvalue.
    StoreUpval(u16),
    /// Pushes a module global.
    LoadGlobal(u32),
    /// Pops into a module global.
    StoreGlobal(u32),
    /// Pushes a prelude value.
    LoadPrelude(u32),
    /// Updates a variable in place: pops the new value and `keys` path keys
    /// (pushed first), then applies `op` at that path.
    Update {
        /// The variable.
        var: VarRef,
        /// How many path keys are on the stack below the value.
        keys: u8,
        /// `=`, `+=`, …
        op: UpdateOp,
    },
    /// Pops `keys` path keys and pushes a reference to `var` at that path.
    MakeRef {
        /// The variable.
        var: VarRef,
        /// How many path keys are on the stack.
        keys: u8,
    },

    /// `value.consts[i]`
    GetField(u32),
    /// `value[key]`
    GetIndex,

    /// `-x`
    Neg,
    /// `!x`
    Not,
    /// `a + b` (the fold)
    Add,
    /// `a - b`
    Sub,
    /// `a * b`
    Mul,
    /// `a / b`
    Div,
    /// `a % b`
    Rem,
    /// `a == b`
    Eq,
    /// `a != b`
    Ne,
    /// `a < b`
    Lt,
    /// `a <= b`
    Le,
    /// `a > b`
    Gt,
    /// `a >= b`
    Ge,
    /// `a..b`
    Range,
    /// Pops `n` pieces and folds them left to right (a glued word).
    Glue(u16),
    /// Pops `n` pieces and concatenates their text (a `"…"` string).
    Interp(u16),

    /// Pops `n` values into a new array.
    MakeArr(u32),
    /// Pops a value and pushes it onto the array below it.
    ArrPush,
    /// Pops an array and appends its items to the array below it (`$%x`).
    ArrExtend,
    /// Pops `n` key/value pairs into a new map.
    MakeMap(u32),

    /// Unconditional jump.
    Jump(i32),
    /// Pops; jumps if falsy.
    JumpIfFalse(i32),
    /// `&&`: jumps (keeping the value) if falsy, else pops.
    JumpIfFalsyKeep(i32),
    /// `||`: jumps (keeping the value) if truthy, else pops.
    JumpIfTruthyKeep(i32),
    /// `??`: jumps (keeping the value) if not null, else pops.
    JumpIfNotNullKeep(i32),

    /// Calls the callee below `n` arguments.
    Call(u16),
    /// Calls the callee below an array of arguments (calls with `$%` splats).
    CallArr,
    /// Returns the top value from the current function.
    Return,

    /// Pushes a fresh accumulator.
    AccBegin,
    /// Pops a value and folds it into the top accumulator.
    AccPut,
    /// Pops the top accumulator onto the operand stack.
    AccEnd,
    /// Pops a value and makes it the top accumulator's content (`clear`).
    AccClear,

    /// Pushes a closure for `protos[i]`.
    MakeClosure(u32),
    /// Pops `n` functions and pushes one overload set.
    MakeOverloads(u16),

    /// Pops the iterable (and the separator, if `split`) and starts iterating.
    IterInit {
        /// Whether a `split` separator is on the stack.
        split: bool,
    },
    /// Pops two whole numbers and starts counting from the first up to the second.
    IterRange,
    /// Pushes the next element (`pairs`: key and element), or jumps when done.
    IterNext {
        /// Whether the loop binds two variables.
        pairs: bool,
        /// Where to jump when the iteration is over.
        exit: i32,
    },
    /// Drops the innermost iterator.
    IterEnd,
    /// Starts a loop (records state for `break`/`continue`).
    LoopEnter,
    /// Ends the innermost loop.
    LoopExit,
    /// Pops a value, unwinds to the innermost loop, folds, and jumps to its exit.
    Break(i32),
    /// Unwinds to the innermost loop and jumps to its next iteration.
    Continue(i32),

    /// Installs an error handler for a `try`.
    PushHandler(i32),
    /// Removes the innermost error handler.
    PopHandler,
    /// Pops a value and raises it as an error.
    Throw,

    /// Pops a module name and pushes the module's map.
    Use,
    /// Runs inline module `modules[i]` and pushes its map.
    MakeModule(u32),
}

/// A captured variable, as seen from the closure being created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    /// Cell `i` of the enclosing frame.
    Cell(u16),
    /// Upvalue `i` of the enclosing closure.
    Upval(u16),
}

/// A compiled function body or module top level.
#[derive(Debug)]
pub struct Proto {
    /// Function name, or `<main>` for a module's top level.
    pub name: Rc<str>,
    /// The instructions.
    pub code: Box<[Op]>,
    /// The source span of each instruction (same length as `code`).
    pub spans: Box<[Span]>,
    /// Constants referenced by `Const` and `GetField`.
    pub consts: Box<[Value]>,
    /// Nested functions referenced by `MakeClosure`.
    pub protos: Box<[Rc<Proto>]>,
    /// Inline modules referenced by `MakeModule`.
    pub modules: Box<[Rc<ModuleProto>]>,
    /// Number of plain slots (parameters first).
    pub n_slots: u16,
    /// Number of cells.
    pub n_cells: u16,
    /// What the closure captures when created.
    pub captures: Box<[Capture]>,
    /// The parameters.
    pub sig: Rc<Signature>,
    /// The definition's source text (for `src`).
    pub source_text: Rc<str>,
    /// Which source the spans refer to.
    pub source: SourceId,
}

/// A compiled module: its top level plus its globals.
#[derive(Debug)]
pub struct ModuleProto {
    /// The top-level code.
    pub main: Rc<Proto>,
    /// Global names (indexed like `LoadGlobal` operands) and which are `pub`.
    pub scope: ModuleScope,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_is_eight_bytes() {
        assert!(
            std::mem::size_of::<Op>() <= 8,
            "{}",
            std::mem::size_of::<Op>()
        );
    }

    #[test]
    fn var_refs_round_trip() {
        for var in [
            Var::Local(0),
            Var::Local(u16::MAX),
            Var::Cell(7),
            Var::Upval(3),
            Var::Global(0),
            Var::Global((1 << 30) - 1),
        ] {
            assert_eq!(VarRef::new(var).get(), var);
        }
    }
}
