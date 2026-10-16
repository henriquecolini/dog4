//! Parameter types and the coercions that typed parameters perform.

use std::fmt;

use super::Value;

/// A set of value types, used by parameter annotations (`x: str|num`, `y: num?`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Type(u8);

impl Type {
    /// `null`
    pub const NULL: Type = Type(1);
    /// `bool`
    pub const BOOL: Type = Type(1 << 1);
    /// `num`
    pub const NUM: Type = Type(1 << 2);
    /// `str`
    pub const STR: Type = Type(1 << 3);
    /// `arr`
    pub const ARR: Type = Type(1 << 4);
    /// `map`
    pub const MAP: Type = Type(1 << 5);
    /// `fn`
    pub const FN: Type = Type(1 << 6);
    /// Any type at all (the default).
    pub const ANY: Type = Type(0x7f);

    const NAMES: [(Type, &'static str); 7] = [
        (Type::NULL, "null"),
        (Type::BOOL, "bool"),
        (Type::NUM, "num"),
        (Type::STR, "str"),
        (Type::ARR, "arr"),
        (Type::MAP, "map"),
        (Type::FN, "fn"),
    ];

    /// The type of `value`.
    pub fn of(value: &Value) -> Type {
        match value {
            Value::Null => Type::NULL,
            Value::Bool(_) => Type::BOOL,
            Value::Int(_) | Value::Float(_) => Type::NUM,
            Value::Str(_) => Type::STR,
            Value::Arr(_) => Type::ARR,
            Value::Map(_) => Type::MAP,
            Value::Func(_) => Type::FN,
            Value::Ref(_) => Type::ANY,
        }
    }

    /// Parses a single type name (`num`, `any`, ...).
    pub fn from_name(name: &str) -> Option<Type> {
        if name == "any" {
            return Some(Type::ANY);
        }
        Type::NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(t, _)| *t)
    }

    /// The union of two types.
    pub fn union(self, other: Type) -> Type {
        Type(self.0 | other.0)
    }

    /// Whether `value` is of this type, without coercion.
    pub fn contains(self, value: &Value) -> bool {
        self.0 & Type::of(value).0 != 0
    }

    /// Whether every type in `other` is also in `self`.
    pub fn includes(self, other: Type) -> bool {
        self.0 & other.0 == other.0
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Type::ANY {
            return f.write_str("any");
        }
        let optional = self.includes(Type::NULL) && *self != Type::NULL;
        let base = if optional {
            Type(self.0 & !Type::NULL.0)
        } else {
            *self
        };
        let names: Vec<&str> = Type::NAMES
            .iter()
            .filter(|(t, _)| base.includes(*t))
            .map(|(_, n)| *n)
            .collect();
        let joined = names.join("|");
        if optional && names.len() > 1 {
            write!(f, "({joined})?")
        } else if optional {
            write!(f, "{joined}?")
        } else {
            f.write_str(&joined)
        }
    }
}

impl fmt::Debug for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Prefixes a type (or any noun) with "a" or "an": `an arr`, `a num`.
pub fn with_article(noun: &str) -> String {
    if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
        format!("an {noun}")
    } else {
        format!("a {noun}")
    }
}

/// A bare word that is a literal *is* that value: `5`, `-2.5`, `007`, `1e3`,
/// `1_000`, `true`, `false` and `null`. Numbers are written as in expressions
/// (see [`number_literal`]); anything else, such as `+5`, `5px` or `inf`, is text.
pub fn bare_literal(text: &str) -> Option<Value> {
    match text {
        "true" => Some(Value::Bool(true)),
        "false" => Some(Value::Bool(false)),
        "null" => Some(Value::Null),
        _ => number_literal(text),
    }
}

/// Parses a number literal: an optional `-`, digits (with `_` separators allowed
/// after the first), an optional fraction `.digits` and an optional exponent
/// `e[+-]digits`. The whole text must be the literal.
pub fn number_literal(text: &str) -> Option<Value> {
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let bytes = body.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let digits = |mut i: usize| {
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
        i
    };
    let is_digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    let mut i = digits(0);
    let mut float = false;
    if bytes.get(i) == Some(&b'.') && is_digit(i + 1) {
        float = true;
        i = digits(i + 1);
    }
    if matches!(bytes.get(i), Some(b'e' | b'E'))
        && (is_digit(i + 1) || (matches!(bytes.get(i + 1), Some(b'+' | b'-')) && is_digit(i + 2)))
    {
        float = true;
        i = digits(i + 2);
    }
    if i != bytes.len() {
        return None;
    }
    let clean: String = body.chars().filter(|&c| c != '_').collect();
    let value = if float {
        Value::num(clean.parse::<f64>().ok()?)
    } else {
        match clean.parse::<i64>() {
            Ok(i) => Value::Int(i),
            Err(_) => Value::num(clean.parse::<f64>().ok()?),
        }
    };
    Some(match (negative, value) {
        (false, value) => value,
        (true, Value::Int(i)) => i.checked_neg().map_or(Value::num(-(i as f64)), Value::Int),
        (true, Value::Float(f)) => Value::num(-f),
        (true, other) => other,
    })
}

/// Parses text as a number the way typed `num` parameters do.
///
/// Accepts integers, decimals, exponents, `inf` and `nan`, ignoring surrounding whitespace.
pub fn parse_num(text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(i) = text.parse::<i64>() {
        return Some(Value::Int(i));
    }
    text.parse::<f64>().ok().map(Value::num)
}

/// Whether [`coerce`] would succeed.
pub fn can_coerce(value: &Value, to: Type) -> bool {
    if to.contains(value) {
        return true;
    }
    (to.includes(Type::NUM) && matches!(value, Value::Str(s) if parse_num(s).is_some()))
        || (to.includes(Type::STR)
            && matches!(value, Value::Int(_) | Value::Float(_) | Value::Bool(_)))
        || (to.includes(Type::ARR) && matches!(value, Value::Str(_)))
        || to.includes(Type::BOOL)
}

/// Converts `value` to the type `to`, trying in order: as is, to `num`, to
/// `str`, to `arr`, to `bool`. Returns the original value on failure.
///
/// * `str` → `num` when the text parses as a number (`"21"` → `21`)
/// * `num`/`bool` → `str` by display (`5` → `"5"`)
/// * `str` → `arr` by splitting on whitespace (`"a b"` → `[a, b]`)
/// * anything → `bool` by truthiness
pub fn coerce(value: Value, to: Type) -> Result<Value, Value> {
    if to.contains(&value) {
        return Ok(value);
    }
    if to.includes(Type::NUM) {
        if let Value::Str(s) = &value {
            if let Some(n) = parse_num(s) {
                return Ok(n);
            }
        }
    }
    if to.includes(Type::STR) && matches!(value, Value::Int(_) | Value::Float(_) | Value::Bool(_)) {
        return Ok(Value::str(value.to_string()));
    }
    if to.includes(Type::ARR) {
        if let Value::Str(s) = &value {
            return Ok(Value::arr(s.split_whitespace().map(Value::from).collect()));
        }
    }
    if to.includes(Type::BOOL) {
        return Ok(Value::Bool(value.truthy()));
    }
    Err(value)
}
