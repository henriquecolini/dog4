//! How values turn into text.
//!
//! Top level (`Display`): strings print raw and `null` prints nothing — this is
//! what `pln`, interpolation and gluing use. Nested (`repr`): dog4 literal
//! syntax, so what a program prints can be pasted back into a program:
//! `pln [a "b c" 5 "5"]` prints `[a "b c" 5 "5"]`, and maps print as
//! `[name = Bob, age = 30]`. Only functions (`<fn name>`) have no literal form.

use std::fmt::{self, Write};

use super::{Value, bare_literal};

/// Values nested deeper than this display as `…` instead of recursing further.
const MAX_DEPTH: usize = 256;

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Str(s) => f.write_str(s),
            Value::Float(x) => {
                let mut out = String::new();
                fmt_num(*x, &mut out);
                f.write_str(&out)
            }
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

/// Appends the nested (dog4 literal) form of `value` to `out`.
pub fn write_repr(value: &Value, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        out.push('…');
        return;
    }
    match value {
        // A bare `null` inside a literal would read back as the text "null".
        Value::Null => out.push_str("(null)"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Float(f) if f.is_nan() => out.push_str("(0 / 0)"),
        Value::Float(f) if f.is_infinite() => {
            out.push_str(if *f > 0.0 { "(1 / 0)" } else { "(-1 / 0)" })
        }
        Value::Float(f) => fmt_num(*f, out),
        Value::Str(s) if reads_back_bare(s) => out.push_str(s),
        Value::Str(s) => write_quoted(s, out),
        Value::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_repr(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::Map(map) if map.is_empty() => out.push_str("[=]"),
        Value::Map(map) => {
            out.push('[');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if is_plain_word(key) {
                    out.push_str(key);
                } else {
                    write_quoted(key, out);
                }
                out.push_str(" = ");
                write_repr(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::Func(f) => {
            let _ = write!(out, "<fn {}>", f.name);
        }
        Value::Ref(_) => out.push_str("<ref>"),
    }
}

/// Whether `s` is a plain word inside `[ … ]`: no separators, quotes, escapes
/// or `$`, and nothing that would start another form (a raw string, a
/// reference, a comment, a control keyword).
fn is_plain_word(s: &str) -> bool {
    !s.is_empty()
        && !s.chars().any(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    ';' | '{' | '}' | '(' | ')' | '[' | ']' | ',' | '=' | '"' | '\\' | '$'
                )
        })
        && !s.starts_with(['\'', '&'])
        && !s.starts_with("//")
        && !s.starts_with("/*")
        && !matches!(s, "if" | "while" | "for" | "try" | "fn")
}

/// Whether a string can be written without quotes and still read back as the
/// same string (not as a number, a bool, or the confusing word `null`).
fn reads_back_bare(s: &str) -> bool {
    is_plain_word(s) && bare_literal(s).is_none() && s != "null"
}

/// Writes `s` as a dog4 double-quoted string.
pub fn write_quoted(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out.push('"');
}
