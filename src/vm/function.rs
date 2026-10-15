//! Functions: signatures, overload sets, script closures and natives.

use std::fmt;
use std::rc::Rc;

use super::Vm;
use super::binding::{Binding, ModuleEnv};
use crate::compile::Proto;
use crate::error::Result;
use crate::value::{Type, Value};

/// A native function. It receives the (already coerced) arguments and may take
/// them out of the slice. Arguments matching a `%rest` parameter follow the
/// positional ones individually (script functions get them as one array).
pub type NativeFn = Rc<dyn Fn(&mut Vm, &mut [Value]) -> Result<Value>>;

/// One parameter of a signature.
#[derive(Clone, Debug, PartialEq)]
pub struct SigParam {
    /// Its name.
    pub name: Rc<str>,
    /// Accepted types (coerced into when not matched exactly).
    pub ty: Type,
    /// Whether this is a `&` reference parameter.
    pub by_ref: bool,
}

/// The parameters of one overload.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    /// Positional parameters.
    pub params: Vec<SigParam>,
    /// The `%rest` parameter, collecting any further arguments into an array.
    pub rest: Option<SigParam>,
    /// Whether `%rest` arguments are collected into one array (script
    /// functions) or passed individually (natives).
    pub(crate) pack_rest: bool,
    /// Arguments pass through untouched: no types or references, and no rest
    /// array to build.
    pub(crate) simple: bool,
}

impl Signature {
    /// Creates a signature.
    pub fn new(params: Vec<SigParam>, rest: Option<SigParam>) -> Signature {
        let mut sig = Signature {
            params,
            rest,
            pack_rest: true,
            simple: false,
        };
        sig.simple = sig.compute_simple();
        sig
    }

    /// The same signature, passing `%rest` arguments individually (for natives).
    pub fn unpacked_rest(mut self) -> Signature {
        self.pack_rest = false;
        self.simple = self.compute_simple();
        self
    }

    fn compute_simple(&self) -> bool {
        let plain = |p: &SigParam| p.ty == Type::ANY && !p.by_ref;
        self.params.iter().all(plain)
            && match &self.rest {
                None => true,
                Some(rest) => !self.pack_rest && plain(rest),
            }
    }

    /// Whether `n` arguments are acceptable.
    pub fn accepts(&self, n: usize) -> bool {
        n == self.params.len() || (self.rest.is_some() && n > self.params.len())
    }

    /// Renders the signature like a declaration: `fn name(a, b: num, %rest)`.
    pub fn display(&self, name: &str) -> String {
        let param = |p: &SigParam, rest: bool| {
            let mut out = String::new();
            if rest {
                out.push('%');
            }
            out.push_str(&p.name);
            if p.by_ref {
                out.push_str(": &");
                if p.ty != Type::ANY {
                    out.push_str(&p.ty.to_string());
                }
            } else if p.ty != Type::ANY {
                out.push_str(": ");
                out.push_str(&p.ty.to_string());
            }
            out
        };
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|p| param(p, false))
            .chain(self.rest.iter().map(|p| param(p, true)))
            .collect();
        format!("fn {name}({})", parts.join(", "))
    }
}

/// A compiled function together with what it captured.
pub struct ScriptFn {
    /// The compiled body.
    pub proto: Rc<Proto>,
    /// Captured variables.
    pub upvals: Box<[Binding]>,
    /// The module whose globals the body sees.
    pub module: Rc<ModuleEnv>,
}

/// What runs when an overload is selected.
#[derive(Clone)]
pub enum Body {
    /// Script code.
    Script(Rc<ScriptFn>),
    /// Rust code.
    Native(NativeFn),
}

/// One overload of a function.
#[derive(Clone)]
pub struct Overload {
    /// Its parameters.
    pub sig: Rc<Signature>,
    /// Its implementation.
    pub body: Body,
}

/// A function value: one or more overloads sharing a name.
pub struct Function {
    /// The name (`<lambda>` for anonymous functions).
    pub name: Rc<str>,
    /// Overloads, in declaration order.
    pub overloads: Vec<Overload>,
}

impl Function {
    /// A function with a single native overload.
    pub fn native(name: &str, sig: Signature, f: NativeFn) -> Function {
        Function {
            name: name.into(),
            overloads: vec![Overload {
                sig: Rc::new(sig),
                body: Body::Native(f),
            }],
        }
    }

    /// The source text of every overload, as `src` prints it.
    pub fn source(&self) -> String {
        let parts: Vec<String> = self
            .overloads
            .iter()
            .map(|o| match &o.body {
                Body::Script(f) => f.proto.source_text.to_string(),
                Body::Native(_) => format!("{} {{\n    // built-in\n}}", o.sig.display(&self.name)),
            })
            .collect();
        parts.join("\n")
    }
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<fn {}>", self.name)
    }
}

impl Value {
    /// Wraps a Rust closure as a dog4 function value. `params` is written in dog4
    /// syntax, e.g. `"chat: num, %text: str"`.
    ///
    /// # Panics
    ///
    /// Panics if `params` doesn't parse; signatures are fixed by the host program.
    pub fn native(
        name: &str,
        params: &str,
        f: impl Fn(&mut Vm, &mut [Value]) -> Result<Value> + 'static,
    ) -> Value {
        let sig = parse_signature(params)
            .unwrap_or_else(|e| panic!("bad signature for native `{name}`: {}", e.message()))
            .unpacked_rest();
        Value::Func(Rc::new(Function::native(name, sig, Rc::new(f))))
    }
}

/// Parses a dog4 parameter list into a [`Signature`].
pub fn parse_signature(params: &str) -> Result<Signature> {
    let (params, rest) = crate::syntax::parse_params(params)?;
    let param = |p: crate::syntax::ast::Param| SigParam {
        name: p.name,
        ty: p.ty,
        by_ref: p.by_ref,
    };
    Ok(Signature::new(
        params.into_iter().map(param).collect(),
        rest.map(param),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_display_like_declarations() {
        let sig = parse_signature("a, b: num, c: &arr, d: &, e: str?, %rest: str|num").unwrap();
        assert_eq!(
            sig.display("f"),
            "fn f(a, b: num, c: &arr, d: &, e: str?, %rest: num|str)"
        );
        assert!(!sig.simple);
        assert!(parse_signature("a, b").unwrap().simple);
        assert!(!parse_signature("%r").unwrap().simple);
        assert!(parse_signature("%r").unwrap().unpacked_rest().simple);
        assert!(!parse_signature("%r: num").unwrap().unpacked_rest().simple);
        assert!(parse_signature("a: wat").is_err());
        assert!(parse_signature("%a, b").is_err());
    }

    #[test]
    fn arity() {
        let sig = parse_signature("a, %rest").unwrap();
        assert!(!sig.accepts(0));
        assert!(sig.accepts(1));
        assert!(sig.accepts(5));
        let fixed = parse_signature("a, b").unwrap();
        assert!(fixed.accepts(2) && !fixed.accepts(3));
    }
}
