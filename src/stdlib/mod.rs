//! The builtin functions, grouped by capability.
//!
//! Every native declares its parameters in dog4 syntax, so builtins get the same
//! overload dispatch and argument coercion as script functions: `add` declares
//! `first: num, %rest: num`, which is why `add 1 2` works on bare words.

mod core;
mod func;
mod iter;
mod json;
mod logic;
mod math;
#[cfg(feature = "net")]
mod net;
mod text;

use std::rc::Rc;

use indexmap::IndexMap;

use crate::error::{Error, Result};
use crate::value::{Map, Value};
use crate::vm::{Body, Function, Overload, Vm, parse_signature};

/// Optional groups of builtins a host can grant or withhold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// `print` and `println`: writing to the runtime's output immediately.
    Print,
    /// `eval`: running code built at runtime.
    Eval,
    /// `random`, `shuffle`, `pick`.
    Random,
    /// `get` and `post`: HTTP requests (needs the `net` feature).
    Net,
}

/// A set of named values, mostly native functions. Defining the same name twice
/// adds an overload.
///
/// Hosts use this to build native modules:
///
/// ```
/// use dog4::{Library, Value};
/// let mut lib = Library::new();
/// lib.def("double", "x: num", |_, args| Ok(Value::num(args[0].as_f64().unwrap() * 2.0)));
/// let module: Value = lib.into_value();
/// ```
#[derive(Default)]
pub struct Library {
    entries: IndexMap<Rc<str>, Entry>,
}

enum Entry {
    Value(Value),
    Overloads(Vec<Overload>),
}

impl Library {
    /// An empty library.
    pub fn new() -> Library {
        Library::default()
    }

    /// Defines (or adds an overload to) a native function.
    ///
    /// # Panics
    ///
    /// Panics if `params` isn't a valid dog4 parameter list.
    pub fn def(
        &mut self,
        name: &str,
        params: &str,
        f: impl Fn(&mut Vm, &mut [Value]) -> Result<Value> + 'static,
    ) -> &mut Library {
        let sig = parse_signature(params)
            .unwrap_or_else(|e| panic!("bad signature for `{name}`: {}", e.message()))
            .unpacked_rest();
        let overload = Overload {
            sig: Rc::new(sig),
            body: Body::Native(Rc::new(f)),
        };
        match self
            .entries
            .entry(name.into())
            .or_insert(Entry::Overloads(Vec::new()))
        {
            Entry::Overloads(overloads) => overloads.push(overload),
            entry => *entry = Entry::Overloads(vec![overload]),
        }
        self
    }

    /// Defines a plain value.
    pub fn value(&mut self, name: &str, value: Value) -> &mut Library {
        self.entries.insert(name.into(), Entry::Value(value));
        self
    }

    /// The entries as `(name, value)` pairs.
    pub fn into_entries(self) -> impl Iterator<Item = (Rc<str>, Value)> {
        self.entries.into_iter().map(|(name, entry)| {
            let value = match entry {
                Entry::Value(v) => v,
                Entry::Overloads(overloads) => Value::Func(Rc::new(Function {
                    name: name.clone(),
                    overloads,
                })),
            };
            (name, value)
        })
    }

    /// The library as a map value (a native module).
    pub fn into_value(self) -> Value {
        let map: Map = self.into_entries().collect();
        Value::map(map)
    }
}

/// Builds the prelude for the given capabilities.
pub(crate) fn prelude(allowed: &dyn Fn(Capability) -> bool) -> Library {
    let mut lib = Library::new();
    core::install(&mut lib);
    iter::install(&mut lib);
    math::install(&mut lib);
    logic::install(&mut lib);
    text::install(&mut lib);
    func::install(&mut lib);
    json::install(&mut lib);
    if allowed(Capability::Print) {
        core::install_print(&mut lib);
    }
    if allowed(Capability::Eval) {
        core::install_eval(&mut lib);
    }
    if allowed(Capability::Random) {
        math::install_random(&mut lib);
    }
    #[cfg(feature = "net")]
    if allowed(Capability::Net) {
        net::install(&mut lib);
    }
    lib
}

// ----- helpers shared by the natives -------------------------------------------------------

/// The number in a (coerced) `num` argument.
pub(crate) fn num(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

/// The text of a (coerced) `str` argument.
pub(crate) fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

/// The items of a (coerced) `arr` argument.
pub(crate) fn items(value: &Value) -> &[Value] {
    match value {
        Value::Arr(items) => items,
        _ => &[],
    }
}

/// Takes an array argument's items, without cloning when uniquely owned.
pub(crate) fn take_items(value: &mut Value) -> Vec<Value> {
    match std::mem::take(value) {
        Value::Arr(items) => items.into_vec(),
        _ => Vec::new(),
    }
}

/// A whole number from a (coerced) `num` argument.
pub(crate) fn int(value: &Value, what: &str) -> Result<i64> {
    match value {
        Value::Int(i) => Ok(*i),
        other => Err(Error::value_error(format!(
            "{what} must be a whole number, not {}",
            other.describe()
        ))),
    }
}

/// Joins values by their display text.
pub(crate) fn join_display(values: &[Value], sep: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        match v {
            Value::Str(s) => out.push_str(s),
            other => {
                let _ = write!(out, "{other}");
            }
        }
    }
    out
}

/// Builds a string value after checking it against the size limit.
pub(crate) fn checked_str(vm: &mut Vm, text: String) -> Result<Value> {
    vm.meter().str_len(text.len())?;
    Ok(Value::str(text))
}
