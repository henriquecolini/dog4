//! Text builtins.

use regex::{Regex, RegexBuilder};

use super::{Library, checked_str, int, join_display, text};
use crate::error::{Error, Result};
use crate::value::Value;
use crate::vm::Vm;

/// A match's groups: `[whole first second …]`, with `null` for groups that
/// didn't take part in the match.
fn groups(captures: &regex::Captures<'_>) -> Value {
    Value::arr(
        captures
            .iter()
            .map(|g| g.map_or(Value::Null, |m| Value::str(m.as_str())))
            .collect(),
    )
}

/// `pad_left` / `pad_right`: `text` widened to `width` characters with `fill`
/// (a space by default) on the left or the right.
fn pad(vm: &mut Vm, args: &mut [Value], left: bool) -> Result<Value> {
    let target = text(&args[0]);
    let width = int(&args[1], "a width")?;
    let fill = match args.get(2) {
        Some(fill) => {
            let mut chars = text(fill).chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c,
                _ => {
                    return Err(Error::value_error(format!(
                        "the fill must be one character, got {}",
                        fill.quoted_repr()
                    )));
                }
            }
        }
        None => ' ',
    };
    let len = target.chars().count();
    let missing = usize::try_from(width).unwrap_or(0).saturating_sub(len);
    if missing == 0 {
        return Ok(std::mem::take(&mut args[0]));
    }
    vm.meter().str_len(
        target
            .len()
            .saturating_add(missing.saturating_mul(fill.len_utf8())),
    )?;
    let padding: String = std::iter::repeat_n(fill, missing).collect();
    let out = if left {
        padding + target
    } else {
        format!("{target}{padding}")
    };
    Ok(Value::str(out))
}

/// Compiles a regex with limits suitable for untrusted patterns. The `regex`
/// crate runs in linear time, so the only risk is a huge compiled program.
fn regex(pattern: &str) -> Result<Regex> {
    if pattern.len() > 4096 {
        return Err(Error::value_error("regular expression too long"));
    }
    RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|e| Error::value_error(format!("invalid regular expression: {e}")))
}

pub(super) fn install(lib: &mut Library) {
    lib.def("upper", "%args", |vm, args| {
        checked_str(vm, join_display(args, " ").to_uppercase())
    });
    lib.def("lower", "%args", |vm, args| {
        checked_str(vm, join_display(args, " ").to_lowercase())
    });
    lib.def("trim", "text: str", |_, args| {
        Ok(Value::str(text(&args[0]).trim()))
    });
    lib.def("trim_start", "text: str", |_, args| {
        Ok(Value::str(text(&args[0]).trim_start()))
    });
    lib.def("trim_end", "text: str", |_, args| {
        Ok(Value::str(text(&args[0]).trim_end()))
    });
    lib.def("pad_left", "text: str, width: num", |vm, args| {
        pad(vm, args, true)
    });
    lib.def(
        "pad_left",
        "text: str, width: num, fill: str",
        |vm, args| pad(vm, args, true),
    );
    lib.def("pad_right", "text: str, width: num", |vm, args| {
        pad(vm, args, false)
    });
    lib.def(
        "pad_right",
        "text: str, width: num, fill: str",
        |vm, args| pad(vm, args, false),
    );
    lib.def("lines", "text: str", |vm, args| {
        let lines: Vec<Value> = text(&args[0]).lines().map(Value::from).collect();
        vm.meter().coll_len(lines.len())?;
        vm.meter().tick(lines.len() as u64)?;
        Ok(Value::arr(lines))
    });
    lib.def("capitalize", "text: str", |vm, args| {
        let mut chars = text(&args[0]).chars();
        let out = match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        };
        checked_str(vm, out)
    });
    lib.def("ord", "char: str", |_, args| {
        let mut chars = text(&args[0]).chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(Value::Int(c as i64)),
            _ => Err(Error::value_error(format!(
                "`ord` needs exactly one character, got {}",
                args[0].quoted_repr()
            ))),
        }
    });
    lib.def("chr", "code: num", |_, args| {
        let code = int(&args[0], "a character code")?;
        u32::try_from(code)
            .ok()
            .and_then(char::from_u32)
            .map(|c| Value::str(c.to_string()))
            .ok_or_else(|| Error::value_error(format!("{code} is not a character code")))
    });
    lib.def("strlen", "text: str", |_, args| {
        Ok(Value::Int(text(&args[0]).chars().count() as i64))
    });
    // dog3's `replace`: every match of a regex, with `$1` / `${name}` in `to`
    // standing for capture groups (`$$` is a literal `$`).
    lib.def(
        "replace",
        "target: str, pattern: str, to: str",
        |vm, args| {
            let re = regex(text(&args[1]))?;
            let (target, to) = (text(&args[0]), text(&args[2]));
            let mut out = String::new();
            let mut last = 0;
            for captures in re.captures_iter(target) {
                let Some(whole) = captures.get(0) else {
                    continue;
                };
                vm.meter().tick(1)?;
                out.push_str(&target[last..whole.start()]);
                captures.expand(to, &mut out);
                last = whole.end();
                vm.meter().str_len(out.len())?;
            }
            out.push_str(&target[last..]);
            checked_str(vm, out)
        },
    );
    lib.def("captures", "text: str, pattern: str", |_, args| {
        let re = regex(text(&args[1]))?;
        Ok(re
            .captures(text(&args[0]))
            .map_or(Value::Null, |c| groups(&c)))
    });
    lib.def("captures_all", "text: str, pattern: str", |vm, args| {
        let re = regex(text(&args[1]))?;
        let mut out = Vec::new();
        for c in re.captures_iter(text(&args[0])) {
            vm.meter().tick(c.len() as u64)?;
            vm.meter().coll_len(out.len() + 1)?;
            out.push(groups(&c));
        }
        Ok(Value::arr(out))
    });
    lib.def("find_all", "text: str, pattern: str", |vm, args| {
        let re = regex(text(&args[1]))?;
        let mut out = Vec::new();
        for m in re.find_iter(text(&args[0])) {
            vm.meter().tick(1)?;
            vm.meter().coll_len(out.len() + 1)?;
            out.push(Value::str(m.as_str()));
        }
        Ok(Value::arr(out))
    });
    lib.def("split_re", "text: str, pattern: str", |vm, args| {
        let re = regex(text(&args[1]))?;
        let target = text(&args[0]);
        if target.is_empty() {
            return Ok(Value::arr(Vec::new()));
        }
        let mut out = Vec::new();
        for piece in re.split(target) {
            vm.meter().tick(1)?;
            vm.meter().coll_len(out.len() + 1)?;
            out.push(Value::str(piece));
        }
        Ok(Value::arr(out))
    });
    lib.def("regex_escape", "text: str", |vm, args| {
        checked_str(vm, regex::escape(text(&args[0])))
    });
    lib.def("starts_with", "text: str, prefix: str", |_, args| {
        Ok(Value::Bool(text(&args[0]).starts_with(text(&args[1]))))
    });
    lib.def("ends_with", "text: str, suffix: str", |_, args| {
        Ok(Value::Bool(text(&args[0]).ends_with(text(&args[1]))))
    });
    // Lines of `target` matching `pattern`, each followed by a newline.
    lib.def("search", "target: str, pattern: str", |vm, args| {
        let re = regex(text(&args[1]))?;
        let mut out = String::new();
        for line in text(&args[0]).lines() {
            vm.meter().tick(1)?;
            if re.is_match(line) {
                out.push_str(line);
                out.push('\n');
            }
        }
        checked_str(vm, out)
    });
    lib.def("matches", "text: str, pattern: str", |_, args| {
        Ok(Value::Bool(regex(text(&args[1]))?.is_match(text(&args[0]))))
    });
    lib.def("is_alpha", "%args", |_, args| {
        let all = args
            .iter()
            .all(|v| v.to_string().chars().all(char::is_alphabetic));
        Ok(Value::Bool(all))
    });
    lib.def("is_digit", "%args", |_, args| {
        let all = args
            .iter()
            .all(|v| v.to_string().chars().all(|c| c.is_ascii_digit()));
        Ok(Value::Bool(all))
    });
    lib.def("is_space", "%args", |_, args| {
        let all = args
            .iter()
            .all(|v| v.to_string().chars().all(char::is_whitespace));
        Ok(Value::Bool(all))
    });
    lib.def("is_alphanumeric", "%args", |_, args| {
        let all = args
            .iter()
            .all(|v| v.to_string().chars().all(char::is_alphanumeric));
        Ok(Value::Bool(all))
    });
}
