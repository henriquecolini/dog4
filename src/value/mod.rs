//! Runtime values: the JSON types plus functions.
//!
//! Values have *value semantics*: arrays and maps are reference counted and
//! copy-on-write, so assigning or passing them is O(1), mutating a uniquely
//! owned one happens in place, and no two variables ever observe each other's
//! changes (explicit `&` references aside).

mod display;
mod json;
mod ops;
mod types;

use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

pub use display::{fmt_num, write_quoted, write_repr};
pub use json::{from_json, parse_json, to_json};
pub use ops::*;
pub use types::{Type, bare_literal, can_coerce, coerce, number_literal, parse_num, with_article};

use crate::vm::{Function, RefData};

/// The map type used by dog4 maps: insertion ordered, string keyed.
pub type Map = IndexMap<Rc<str>, Value, FxBuildHasher>;

/// A dog4 value.
#[derive(Clone, Default)]
pub enum Value {
    /// Nothing. The identity of `+`; what an empty block evaluates to.
    #[default]
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number with an integral value that fits in an `i64`.
    Int(i64),
    /// Any other number. Constructors normalise integral floats to [`Value::Int`].
    Float(f64),
    /// UTF-8 text.
    Str(Rc<String>),
    /// An ordered list.
    Arr(Array),
    /// A string-keyed, insertion-ordered map.
    Map(Object),
    /// A function (closure, native, or overload set).
    Func(Rc<Function>),
    /// A reference to a variable. Only ever exists transiently while passing
    /// `&x` arguments to a call; scripts never see one as a value.
    #[doc(hidden)]
    Ref(Rc<RefData>),
}

/// A copy-on-write array. Dropping deeply nested arrays never overflows the stack.
#[derive(Clone, Default)]
pub struct Array(Rc<Vec<Value>>);

/// A copy-on-write map. Dropping deeply nested maps never overflows the stack.
#[derive(Clone, Default)]
pub struct Object(Rc<Map>);

impl Array {
    /// Wraps a vector.
    pub fn new(items: Vec<Value>) -> Array {
        Array(Rc::new(items))
    }

    /// Mutable access, cloning first if shared.
    pub fn make_mut(&mut self) -> &mut Vec<Value> {
        Rc::make_mut(&mut self.0)
    }

    /// Takes the items out, cloning only if shared.
    pub fn into_vec(mut self) -> Vec<Value> {
        match Rc::get_mut(&mut self.0) {
            Some(items) => std::mem::take(items),
            None => (*self.0).clone(),
        }
    }

    /// Whether both arrays share the same allocation.
    pub fn ptr_eq(&self, other: &Array) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Object {
    /// Wraps a map.
    pub fn new(map: Map) -> Object {
        Object(Rc::new(map))
    }

    /// Mutable access, cloning first if shared.
    pub fn make_mut(&mut self) -> &mut Map {
        Rc::make_mut(&mut self.0)
    }

    /// Takes the entries out, cloning only if shared.
    pub fn into_map(mut self) -> Map {
        match Rc::get_mut(&mut self.0) {
            Some(map) => std::mem::take(map),
            None => (*self.0).clone(),
        }
    }
}

impl Deref for Array {
    type Target = Vec<Value>;
    fn deref(&self) -> &Vec<Value> {
        &self.0
    }
}

impl Deref for Object {
    type Target = Map;
    fn deref(&self) -> &Map {
        &self.0
    }
}

impl Drop for Array {
    fn drop(&mut self) {
        if let Some(items) = Rc::get_mut(&mut self.0) {
            if items.iter().any(Value::is_container) {
                drop_iteratively(std::mem::take(items));
            }
        }
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        if let Some(map) = Rc::get_mut(&mut self.0) {
            if map.values().any(Value::is_container) {
                drop_iteratively(map.drain(..).map(|(_, v)| v).collect());
            }
        }
    }
}

/// Drops a tree of values using an explicit stack instead of recursion.
fn drop_iteratively(mut stack: Vec<Value>) {
    while let Some(value) = stack.pop() {
        match value {
            Value::Arr(mut arr) => {
                if let Some(items) = Rc::get_mut(&mut arr.0) {
                    stack.append(items);
                }
            }
            Value::Map(mut obj) => {
                if let Some(map) = Rc::get_mut(&mut obj.0) {
                    stack.extend(map.drain(..).map(|(_, v)| v));
                }
            }
            _ => {}
        }
    }
}

impl Value {
    /// A string value.
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(Rc::new(s.into()))
    }

    /// A number, normalised to [`Value::Int`] when integral and in range.
    pub fn num(f: f64) -> Value {
        if f.fract() == 0.0
            && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&f)
        {
            Value::Int(f as i64)
        } else {
            Value::Float(f)
        }
    }

    /// An array value.
    pub fn arr(items: Vec<Value>) -> Value {
        Value::Arr(Array::new(items))
    }

    /// A map value.
    pub fn map(map: Map) -> Value {
        Value::Map(Object::new(map))
    }

    /// The name of this value's type, as scripts see it (`type $x`).
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) | Value::Float(_) => "num",
            Value::Str(_) => "str",
            Value::Arr(_) => "arr",
            Value::Map(_) => "map",
            Value::Func(_) => "fn",
            Value::Ref(_) => "ref",
        }
    }

    /// Truthiness: `false null 0 "" [] [=]` are falsy, everything else is truthy.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Arr(a) => !a.is_empty(),
            Value::Map(m) => !m.is_empty(),
            Value::Func(_) | Value::Ref(_) => true,
        }
    }

    /// The numeric value, if this is a number.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Int(i) => Some(i as f64),
            Value::Float(f) => Some(f),
            _ => None,
        }
    }

    /// The integer value, if this is a whole number.
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Value::Int(i) => Some(i),
            _ => None,
        }
    }

    /// The text, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Whether this is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Drops a value without calling the (out-of-line) drop glue when there is
    /// nothing to free. The VM's hot paths discard many numbers and bools.
    #[inline(always)]
    pub fn discard(self) {
        match self {
            Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => {
                std::mem::forget(self)
            }
            heap => drop(heap),
        }
    }

    fn is_container(&self) -> bool {
        matches!(self, Value::Arr(_) | Value::Map(_))
    }

    /// A short description for error messages: the type, plus the value when small.
    pub fn describe(&self) -> String {
        match self {
            Value::Str(s) if s.chars().count() <= 24 => format!("str {}", self.quoted_repr()),
            Value::Str(_) => "a long str".into(),
            Value::Int(_) | Value::Float(_) | Value::Bool(_) => {
                format!("{} {}", self.type_name(), self.repr())
            }
            other => other.type_name().into(),
        }
    }

    /// The nested form: dog4 literal syntax (`[a "b c" 5]`, `[k = v]`), with
    /// strings quoted only when they need to be.
    pub fn repr(&self) -> String {
        let mut out = String::new();
        write_repr(self, &mut out, 0);
        out
    }

    /// Like [`repr`](Value::repr), but a string is always quoted, which keeps
    /// error messages and listings unambiguous.
    pub fn quoted_repr(&self) -> String {
        match self {
            Value::Str(s) => {
                let mut out = String::new();
                write_quoted(s, &mut out);
                out
            }
            other => other.repr(),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
                self.as_f64() == other.as_f64()
            }
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Arr(a), Value::Arr(b)) => a.ptr_eq(b) || **a == **b,
            (Value::Map(a), Value::Map(b)) => Rc::ptr_eq(&a.0, &b.0) || **a == **b,
            (Value::Func(a), Value::Func(b)) => Rc::ptr_eq(a, b),
            (Value::Ref(a), Value::Ref(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.repr())
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Value {
        Value::Int(i)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Value {
        Value::num(f)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::str(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::str(s)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Value {
        Value::arr(items)
    }
}

impl From<Map> for Value {
    fn from(map: Map) -> Value {
        Value::map(map)
    }
}

#[cfg(test)]
mod tests;
