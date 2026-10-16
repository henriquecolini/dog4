//! JSON: the `json` module value, plus dog3's `gron`.

use std::fmt::Write;

use super::{Library, checked_str, text};
use crate::value::{Value, parse_json, to_json};

pub(super) fn install(lib: &mut Library) {
    let mut json = Library::new();
    json.def("parse", "text: str", |_, args| parse_json(text(&args[0])));
    json.def("encode", "value", |vm, args| {
        let json = to_json(&args[0])?;
        checked_str(vm, json.to_string())
    });
    json.def("pretty", "value", |vm, args| {
        let json = to_json(&args[0])?;
        checked_str(vm, serde_json::to_string_pretty(&json).unwrap_or_default())
    });
    lib.module("json", json);
    // A flattened, grep-able listing of a JSON document (keys sorted, like gron).
    lib.def("gron", "input: str", |vm, args| {
        let Ok(value) = parse_json(text(&args[0])) else {
            return Ok(Value::Null);
        };
        let mut out = String::new();
        flatten(&value, "json", &mut out);
        checked_str(vm, out)
    });
}

fn flatten(value: &Value, path: &str, out: &mut String) {
    match value {
        Value::Map(map) => {
            let _ = writeln!(out, "{path} = {{}}");
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                let simple = key.starts_with(|c: char| c.is_alphabetic() || c == '_')
                    && key.chars().all(|c| c.is_alphanumeric() || c == '_');
                let child = if simple {
                    format!("{path}.{key}")
                } else {
                    format!("{path}[{}]", serde_json::Value::from(&**key))
                };
                flatten(&map[key], &child, out);
            }
        }
        Value::Arr(items) => {
            let _ = writeln!(out, "{path} = []");
            for (i, item) in items.iter().enumerate() {
                flatten(item, &format!("{path}[{i}]"), out);
            }
        }
        scalar => {
            // gron lines are JSON; values here came from JSON, so they encode.
            let json = to_json(scalar).unwrap_or(serde_json::Value::Null);
            let _ = writeln!(out, "{path} = {json}");
        }
    }
}
