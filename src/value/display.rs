//! How values turn into text.
//!
//! Top level (`Display`): strings print raw and `null` prints nothing — this is
//! what `pln`, interpolation and gluing use. Nested (`repr`): JSON-like, strings
//! quoted, so `pln [a "b c"]` prints `["a", "b c"]`.

use std::fmt::{self, Write};

use super::Value;

/// Values nested deeper than this display as `…` instead of recursing further.
const MAX_DEPTH: usize = 256;

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Str(s) => f.write_str(s),
            other => {
                let mut out = String::new();
                write_repr(other, &mut out, 0);
                f.write_str(&out)
            }
        }
    }
}

/// Formats a number: integers plainly, floats shortest round-trip, with an
/// exponent only for very large or very small magnitudes.
pub fn fmt_num(f: f64, out: &mut String) {
    if f.is_nan() {
        out.push_str("NaN");
    } else if f.is_infinite() {
        out.push_str(if f > 0.0 { "inf" } else { "-inf" });
    } else if f != 0.0 && (f.abs() >= 1e21 || f.abs() < 1e-7) {
        let _ = write!(out, "{f:e}");
    } else {
        let _ = write!(out, "{f}");
    }
}

/// Appends the nested form of `value` to `out`.
pub fn write_repr(value: &Value, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        out.push('…');
        return;
    }
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Float(f) => fmt_num(*f, out),
        Value::Str(s) => write_quoted(s, out),
        Value::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_repr(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::Map(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_quoted(key, out);
                out.push_str(": ");
                write_repr(item, out, depth + 1);
            }
            out.push('}');
        }
        Value::Func(f) => {
            let _ = write!(out, "<fn {}>", f.name);
        }
        Value::Ref(_) => out.push_str("<ref>"),
    }
}

/// Writes `s` as a JSON string literal.
fn write_quoted(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
