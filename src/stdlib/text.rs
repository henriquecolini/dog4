//! Text builtins.

use regex::{Regex, RegexBuilder};

use super::{Library, checked_str, join_display, text};
use crate::error::{Error, Result};
use crate::value::Value;

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
    lib.def("strlen", "text: str", |_, args| {
        Ok(Value::Int(text(&args[0]).chars().count() as i64))
    });
    lib.def("replace", "target: str, from: str, to: str", |vm, args| {
        let (target, from, to) = (text(&args[0]), text(&args[1]), text(&args[2]));
        if from.is_empty() {
            return Ok(std::mem::take(&mut args[0]));
        }
        let count = target.matches(from).count();
        vm.meter().str_len(target.len() + count * to.len())?;
        Ok(Value::str(target.replace(from, to)))
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
    lib.def("is_alphanumeric", "%args", |_, args| {
        let all = args
            .iter()
            .all(|v| v.to_string().chars().all(char::is_alphanumeric));
        Ok(Value::Bool(all))
    });
}
