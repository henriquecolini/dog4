//! The single error type used by every stage: parsing, compiling and running.

use std::fmt;
use std::rc::Rc;

use crate::syntax::{Location, SourceId, SourceMap, Span};
use crate::value::Value;

/// What went wrong, broadly. Exposed to scripts as the `kind` of a caught error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The source text could not be parsed.
    Syntax,
    /// A name was used that doesn't exist (or isn't initialized yet).
    Name,
    /// A value had the wrong type for an operation.
    Type,
    /// No overload accepts the given number of arguments.
    Arity,
    /// A value was the right type but unusable (bad regex, bad index, ...).
    Value,
    /// Raised by `throw` / `fail` / `assert`.
    Thrown,
    /// A module could not be found or loaded.
    Module,
    /// An I/O or network operation failed.
    Io,
    /// A sandbox limit was hit. These cannot be caught by `try`.
    Limit(Limit),
}

/// Which sandbox limit was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// The instruction budget ran out.
    Fuel,
    /// The wall-clock deadline passed.
    Timeout,
    /// The host requested an interrupt.
    Interrupted,
    /// Too many nested function calls.
    CallDepth,
    /// Too many nested native → script callbacks.
    NativeDepth,
    /// A string would exceed the maximum size.
    StrSize,
    /// An array or map would exceed the maximum length.
    CollectionSize,
    /// Too much text was printed.
    Output,
    /// Source code (or a value being displayed) is nested too deeply.
    Nesting,
}

impl ErrorKind {
    /// The name scripts see in a caught error's `kind` field.
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Syntax => "syntax",
            ErrorKind::Name => "name",
            ErrorKind::Type => "type",
            ErrorKind::Arity => "arity",
            ErrorKind::Value => "value",
            ErrorKind::Thrown => "throw",
            ErrorKind::Module => "module",
            ErrorKind::Io => "io",
            ErrorKind::Limit(_) => "limit",
        }
    }

    /// Whether `try` may catch this error. Limit errors are never catchable so a
    /// sandboxed script can't swallow its own termination.
    pub fn is_catchable(self) -> bool {
        !matches!(self, ErrorKind::Limit(_))
    }
}

/// One frame of a script stack trace.
#[derive(Clone, Debug)]
pub struct TraceEntry {
    /// Function name (`<main>` for module top level).
    pub function: Rc<str>,
    /// Where the call happened, if known.
    pub loc: Option<(SourceId, Span)>,
    resolved: Option<Location>,
}

/// An error from any stage of dog4.
///
/// Errors returned by [`Runtime`](crate::Runtime) methods have their locations resolved,
/// so `to_string()` gives a complete, human-friendly report.
pub struct Error(Box<Inner>);

struct Inner {
    kind: ErrorKind,
    message: String,
    loc: Option<(SourceId, Span)>,
    value: Value,
    trace: Vec<TraceEntry>,
    resolved: Option<Location>,
}

/// Shorthand for results whose error is [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Creates an error with no location.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error(Box::new(Inner {
            kind,
            message: message.into(),
            loc: None,
            value: Value::Null,
            trace: Vec::new(),
            resolved: None,
        }))
    }

    /// A type error.
    pub fn type_error(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Type, message)
    }

    /// A value error.
    pub fn value_error(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Value, message)
    }

    /// A name error.
    pub fn name_error(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Name, message)
    }

    /// A limit error.
    pub fn limit(limit: Limit) -> Error {
        let message = match limit {
            Limit::Fuel => "ran out of fuel (the script did too much work)",
            Limit::Timeout => "the script took too long",
            Limit::Interrupted => "the script was interrupted",
            Limit::CallDepth => "too many nested function calls",
            Limit::NativeDepth => "too many nested callbacks",
            Limit::StrSize => "string too large",
            Limit::CollectionSize => "array or map too large",
            Limit::Output => "printed too much output",
            Limit::Nesting => "nested too deeply",
        };
        Error::new(ErrorKind::Limit(limit), message)
    }

    /// An error raised by script code carrying an arbitrary value.
    pub fn thrown(value: Value) -> Error {
        let mut err = Error::new(ErrorKind::Thrown, value.to_string());
        err.0.value = value;
        err
    }

    /// Attaches a location if the error doesn't have one yet.
    pub fn at(mut self, source: SourceId, span: Span) -> Error {
        if self.0.loc.is_none() {
            self.0.loc = Some((source, span));
        }
        self
    }

    /// The error's kind.
    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    /// The message without location information.
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// The value passed to `throw` (or `null`).
    pub fn value(&self) -> &Value {
        &self.0.value
    }

    /// The raw location, if any.
    pub fn loc(&self) -> Option<(SourceId, Span)> {
        self.0.loc
    }

    /// The resolved location, available on errors returned by the runtime.
    pub fn location(&self) -> Option<&Location> {
        self.0.resolved.as_ref()
    }

    /// The script stack trace (innermost call first).
    pub fn trace(&self) -> &[TraceEntry] {
        &self.0.trace
    }

    pub(crate) fn push_trace(&mut self, function: Rc<str>, loc: Option<(SourceId, Span)>) {
        self.0.trace.push(TraceEntry {
            function,
            loc,
            resolved: None,
        });
    }

    /// Resolves raw spans into lines and columns so the error renders standalone.
    pub(crate) fn resolve(mut self, sources: &SourceMap) -> Error {
        if let Some((id, span)) = self.0.loc {
            self.0.resolved = Some(sources.locate(id, span));
        }
        for entry in &mut self.0.trace {
            if let Some((id, span)) = entry.loc {
                entry.resolved = Some(sources.locate(id, span));
            }
        }
        self
    }

    /// The error as a script-visible map: `[message = …, kind = …, value = …]`.
    pub fn to_value(&self) -> Value {
        let mut map = crate::value::Map::default();
        map.insert("message".into(), Value::str(self.message()));
        map.insert("kind".into(), Value::str(self.kind().name()));
        map.insert("value".into(), self.value().clone());
        if let Some(loc) = self.location() {
            map.insert("line".into(), Value::Int(loc.line as i64));
        }
        Value::map(map)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error: {}", self.0.message)?;
        if let Some(loc) = &self.0.resolved {
            let gutter = loc.line.to_string().len();
            let pad = " ".repeat(gutter);
            let prefix: String = loc
                .line_text
                .chars()
                .take(loc.col as usize - 1)
                .map(|c| if c == '\t' { '\t' } else { ' ' })
                .collect();
            write!(
                f,
                "\n{pad}--> {}:{}:{}\n{pad} |\n{} | {}\n{pad} | {}{}",
                loc.file,
                loc.line,
                loc.col,
                loc.line,
                loc.line_text,
                prefix,
                "^".repeat(loc.width as usize)
            )?;
        }
        const HEAD: usize = 6;
        const TAIL: usize = 3;
        let trace = &self.0.trace;
        for (i, entry) in trace.iter().enumerate() {
            if trace.len() > HEAD + TAIL + 1 && i >= HEAD && i < trace.len() - TAIL {
                if i == HEAD {
                    write!(f, "\n  … {} more calls", trace.len() - HEAD - TAIL)?;
                }
                continue;
            }
            match &entry.resolved {
                Some(loc) => write!(
                    f,
                    "\n  in {} ({}:{}:{})",
                    entry.function, loc.file, loc.line, loc.col
                )?,
                None => write!(f, "\n  in {}", entry.function)?,
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error({:?}, {:?})", self.0.kind, self.0.message)
    }
}

impl std::error::Error for Error {}
