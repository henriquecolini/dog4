//! # dog4
//!
//! An esoteric, Bash-flavoured scripting language where every statement
//! *outputs* into its enclosing block, and a block's value is everything its
//! statements output, folded together with `+`:
//!
//! ```
//! let mut rt = dog4::Runtime::new();
//! let total = rt.run("main", "put for p in [1 2 3] { put ($p * 10) }").unwrap();
//! assert_eq!(total.to_string(), "60");
//! ```
//!
//! The pipeline is source → [`syntax`] (hand-written parser) → [`compile`]
//! (bytecode) → [`vm`] (stack machine). Embedders use [`Runtime`], configure it
//! with [`Builder`] (limits, capabilities, a [`ModuleProvider`]) and extend it
//! with native functions via [`Library`].

#![warn(missing_docs)]

pub mod compile;
mod error;
mod limits;
pub mod module;
mod runtime;
mod stdlib;
pub mod syntax;
pub mod value;
pub mod vm;

pub use error::{Error, ErrorKind, Limit, Result, TraceEntry};
pub use limits::{Interrupt, Limits, Meter};
pub use module::{
    ChainProvider, FsProvider, MemoryProvider, ModuleProvider, ModuleSource, ProviderError,
};
pub use runtime::{Builder, OutputBuffer, Runtime};
pub use stdlib::{Capability, Library};
pub use value::{Map, Type, Value};
pub use vm::Vm;
