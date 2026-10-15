//! Human-readable bytecode listings, for `dog4 --disasm` and tests.

use std::fmt::Write;

use super::bytecode::{Op, Proto};

/// Disassembles a proto and, recursively, its nested functions and modules.
pub fn disassemble(proto: &Proto) -> String {
    let mut out = String::new();
    write_proto(proto, &mut out, "");
    out
}

fn write_proto(proto: &Proto, out: &mut String, indent: &str) {
    let _ = writeln!(
        out,
        "{indent}fn {} (slots {}, cells {}, captures {:?})",
        proto.name, proto.n_slots, proto.n_cells, proto.captures
    );
    for (i, op) in proto.code.iter().enumerate() {
        let _ = write!(out, "{indent}  {i:4}  ");
        match *op {
            Op::Const(c) | Op::GetField(c) => {
                let _ = write!(out, "{op:?}  ; {}", proto.consts[c as usize].quoted_repr());
            }
            Op::Jump(o)
            | Op::JumpIfFalse(o)
            | Op::JumpIfFalsyKeep(o)
            | Op::JumpIfTruthyKeep(o)
            | Op::JumpIfNotNullKeep(o)
            | Op::Break(o)
            | Op::Continue(o)
            | Op::PushHandler(o)
            | Op::IterNext { exit: o, .. } => {
                let _ = write!(out, "{op:?}  ; -> {}", i as i64 + 1 + o as i64);
            }
            _ => {
                let _ = write!(out, "{op:?}");
            }
        }
        out.push('\n');
    }
    let nested = format!("{indent}    ");
    for child in proto.protos.iter() {
        write_proto(child, out, &nested);
    }
    for module in proto.modules.iter() {
        let _ = writeln!(out, "{nested}module (globals {:?})", module.globals);
        write_proto(&module.main, out, &nested);
    }
}
