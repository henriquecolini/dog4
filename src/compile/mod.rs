//! Syntax tree → bytecode.

pub mod bytecode;
mod compiler;
mod disasm;

pub use bytecode::{Capture, ModuleProto, Op, Proto, Var, VarRef};
pub use compiler::{ModuleScope, PreludeNames, compile_module};
pub use disasm::disassemble;
