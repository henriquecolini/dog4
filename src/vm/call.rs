//! Calling functions: overload selection, coercion, and `%rest` collection.

use super::{Body, FrameKind, Function, Signature, Vm};
use crate::error::{Error, ErrorKind, Result};
use crate::limits::Meter;
use crate::value::{Value, can_coerce, coerce, with_article};

/// How well arguments fit an overload.
enum Fit {
    /// Every argument already has an accepted type.
    Exact,
    /// Some arguments need coercion.
    Coerced,
    /// They don't fit, and this is why.
    No(Error),
}

impl Vm {
    /// Calls the callee sitting below `argc` arguments on the stack.
    ///
    /// Natives run immediately and leave their result on the stack (`false`);
    /// script functions get a new frame (`true`).
    pub(crate) fn call_value(&mut self, argc: usize) -> Result<bool> {
        self.meter.tick(1)?;
        let callee_at = self.stack.len() - argc - 1;
        let function = match &self.stack[callee_at] {
            Value::Func(f) => f.clone(),
            other => {
                return Err(Error::type_error(format!(
                    "{} is not a function",
                    other.describe()
                )));
            }
        };
        let args_at = callee_at + 1;
        let index = select(&function, &self.stack[args_at..])?;
        let overload = &function.overloads[index];
        prepare_args(&overload.sig, &mut self.stack, args_at, &mut self.meter)?;
        match &overload.body {
            Body::Script(script) => {
                let script = script.clone();
                self.push_frame(script, args_at, FrameKind::Call)?;
                Ok(true)
            }
            Body::Native(native) => {
                let native = native.clone();
                let mut args: Vec<Value> = self.stack.drain(args_at..).collect();
                self.stack.pop();
                let result = native(self, &mut args)?;
                self.stack.push(result);
                Ok(false)
            }
        }
    }
}

/// Picks the overload to call: the first exact match, else the first that
/// matches with coercion.
fn select(function: &Function, args: &[Value]) -> Result<usize> {
    let n = args.len();
    if let [only] = function.overloads.as_slice() {
        if !only.sig.accepts(n) {
            return Err(arity_error(function, n));
        }
        if only.sig.simple {
            return Ok(0);
        }
        return match fit(&function.name, &only.sig, args) {
            Fit::Exact | Fit::Coerced => Ok(0),
            Fit::No(err) => Err(err),
        };
    }
    let mut coerced = None;
    let mut candidates = Vec::new();
    for (i, overload) in function.overloads.iter().enumerate() {
        if !overload.sig.accepts(n) {
            continue;
        }
        match fit(&function.name, &overload.sig, args) {
            Fit::Exact => return Ok(i),
            Fit::Coerced => {
                coerced.get_or_insert(i);
            }
            Fit::No(err) => candidates.push(err),
        }
    }
    if let Some(i) = coerced {
        return Ok(i);
    }
    match candidates.len() {
        0 => Err(arity_error(function, n)),
        1 => Err(candidates.pop().unwrap_or_else(|| unreachable!())),
        _ => {
            let types: Vec<&str> = args.iter().map(|a| deref_type(a)).collect();
            Err(Error::type_error(format!(
                "no overload of `{}` accepts ({}); it has {}",
                function.name,
                types.join(", "),
                list_overloads(function)
            )))
        }
    }
}

fn deref_type(arg: &Value) -> &'static str {
    match arg {
        Value::Ref(r) => r.read().map(|v| v.type_name()).unwrap_or("ref"),
        other => other.type_name(),
    }
}

fn list_overloads(function: &Function) -> String {
    let sigs: Vec<String> = function
        .overloads
        .iter()
        .map(|o| o.sig.display(&function.name))
        .collect();
    sigs.join(", ")
}

fn arity_error(function: &Function, n: usize) -> Error {
    let plural = |k: usize| if k == 1 { "argument" } else { "arguments" };
    let message = match function.overloads.as_slice() {
        [only] if only.sig.rest.is_some() => {
            let min = only.sig.params.len();
            format!(
                "`{}` takes at least {min} {}, got {n}",
                function.name,
                plural(min)
            )
        }
        [only] => {
            let k = only.sig.params.len();
            format!("`{}` takes {k} {}, got {n}", function.name, plural(k))
        }
        _ => format!(
            "no overload of `{}` takes {n} {}; it has {}",
            function.name,
            plural(n),
            list_overloads(function)
        ),
    };
    Error::new(ErrorKind::Arity, message)
}

fn fit(name: &str, sig: &Signature, args: &[Value]) -> Fit {
    let mut exact = true;
    for (i, arg) in args.iter().enumerate() {
        let param = sig.params.get(i).or(sig.rest.as_ref());
        let Some(param) = param else {
            return Fit::No(Error::new(
                ErrorKind::Arity,
                format!("`{name}` got too many arguments"),
            ));
        };
        if param.by_ref {
            let Value::Ref(r) = arg else {
                return Fit::No(Error::type_error(format!(
                    "`{}` of `{name}` is a reference parameter; pass `&variable`",
                    param.name
                )));
            };
            match r.read() {
                Ok(v) if param.ty.contains(&v) => continue,
                Ok(v) => {
                    return Fit::No(Error::type_error(format!(
                        "`{name}` expects a reference to {} for `{}`, got one to {}",
                        with_article(&param.ty.to_string()),
                        param.name,
                        v.describe()
                    )));
                }
                Err(e) => return Fit::No(e),
            }
        }
        let owned;
        let value = match arg {
            Value::Ref(r) => match r.read() {
                Ok(v) => {
                    owned = v;
                    &owned
                }
                Err(e) => return Fit::No(e),
            },
            other => other,
        };
        if param.ty.contains(value) {
            continue;
        }
        if can_coerce(value, param.ty) {
            exact = false;
            continue;
        }
        return Fit::No(Error::type_error(format!(
            "`{name}` expects {} for `{}`, got {}",
            with_article(&param.ty.to_string()),
            param.name,
            value.describe()
        )));
    }
    if exact { Fit::Exact } else { Fit::Coerced }
}

/// Dereferences, coerces and collects the arguments in place for `sig`.
fn prepare_args(
    sig: &Signature,
    stack: &mut Vec<Value>,
    at: usize,
    meter: &mut Meter,
) -> Result<()> {
    if sig.simple {
        for arg in &mut stack[at..] {
            if let Value::Ref(r) = arg {
                *arg = r.read()?;
            }
        }
        return Ok(());
    }
    for (i, param) in sig.params.iter().enumerate() {
        let slot = &mut stack[at + i];
        if param.by_ref {
            continue;
        }
        if let Value::Ref(r) = slot {
            *slot = r.read()?;
        }
        if !param.ty.contains(slot) {
            let value = std::mem::take(slot);
            *slot = coerce(value, param.ty).map_err(|v| {
                Error::type_error(format!("can't convert {} to {}", v.describe(), param.ty))
            })?;
        }
    }
    if let Some(rest) = &sig.rest {
        let extra: Vec<Value> = stack.drain(at + sig.params.len()..).collect();
        meter.coll_len(extra.len())?;
        let mut items = Vec::with_capacity(extra.len());
        for value in extra {
            let value = match value {
                Value::Ref(r) => r.read()?,
                other => other,
            };
            items.push(coerce(value, rest.ty).map_err(|v| {
                Error::type_error(format!("can't convert {} to {}", v.describe(), rest.ty))
            })?);
        }
        stack.push(Value::arr(items));
    }
    Ok(())
}
