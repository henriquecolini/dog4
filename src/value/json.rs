//! Conversion between dog4 values and JSON.

use super::{Map, Value};
use crate::error::{Error, Limit, Result};

/// Nesting deeper than this is refused when encoding.
const MAX_DEPTH: usize = 256;

/// Converts parsed JSON into a value.
pub fn from_json(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::num(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => Value::str(s),
        serde_json::Value::Array(items) => Value::arr(items.into_iter().map(from_json).collect()),
        serde_json::Value::Object(entries) => {
            let map: Map = entries
                .into_iter()
                .map(|(k, v)| (k.into(), from_json(v)))
                .collect();
            Value::map(map)
        }
    }
}

/// Parses JSON text. `serde_json` refuses input nested deeper than 128 levels,
/// which keeps hostile input from overflowing the stack.
pub fn parse_json(text: &str) -> Result<Value> {
    serde_json::from_str::<serde_json::Value>(text)
        .map(from_json)
        .map_err(|e| Error::value_error(format!("invalid JSON: {e}")))
}

/// Converts a value to JSON. Functions and non-finite numbers have no JSON form.
pub fn to_json(value: &Value) -> Result<serde_json::Value> {
    encode(value, 0)
}

fn encode(value: &Value, depth: usize) -> Result<serde_json::Value> {
    if depth > MAX_DEPTH {
        return Err(Error::limit(Limit::Nesting));
    }
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| Error::value_error(format!("{f} has no JSON form")))?,
        Value::Str(s) => serde_json::Value::String(s.to_string()),
        Value::Arr(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|v| encode(v, depth + 1))
                .collect::<Result<_>>()?,
        ),
        Value::Map(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.to_string(), encode(v, depth + 1)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Func(_) | Value::Ref(_) => {
            return Err(Error::type_error(format!(
                "{} has no JSON form",
                super::with_article(value.type_name())
            )));
        }
    })
}
