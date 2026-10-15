//! Source text → syntax tree.

pub mod ast;
mod expr;
mod parser;
mod span;

pub use parser::{parse, parse_params};
pub use span::{Location, Source, SourceId, SourceMap, Span};

#[cfg(test)]
mod tests;
