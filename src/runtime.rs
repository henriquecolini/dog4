//! The embedding API.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use rand::SeedableRng;
use rand::rngs::SmallRng;
use rustc_hash::FxHashSet;

use crate::compile::{ModuleScope, PreludeNames, compile_module, disassemble};
use crate::error::{Error, Result};
use crate::limits::{Interrupt, Limits};
use crate::module::ModuleProvider;
use crate::stdlib::{self, Capability};
use crate::syntax::parse;
use crate::value::Value;
use crate::vm::{ModuleEnv, Vm};

/// Configures and creates a [`Runtime`].
#[derive(Default)]
pub struct Builder {
    limits: Limits,
    denied: FxHashSet<Capability>,
    provider: Option<Box<dyn ModuleProvider>>,
    globals: Vec<(Rc<str>, Value)>,
    modules: Vec<(Rc<str>, Value)>,
    output: Option<Box<dyn Write>>,
    seed: Option<u64>,
}

impl Builder {
    /// A trusted configuration: every capability, generous limits.
    pub fn new() -> Builder {
        Builder::default()
    }

    /// A configuration for untrusted code: [`Limits::sandbox`] and no network.
    /// Remember to also give it an [`output`](Builder::output) you control.
    pub fn sandboxed() -> Builder {
        Builder::new()
            .limits(Limits::sandbox())
            .allow(Capability::Net, false)
    }

    /// Sets the resource limits.
    pub fn limits(mut self, limits: Limits) -> Builder {
        self.limits = limits;
        self
    }

    /// Grants or withholds a group of builtins.
    pub fn allow(mut self, capability: Capability, allowed: bool) -> Builder {
        if allowed {
            self.denied.remove(&capability);
        } else {
            self.denied.insert(capability);
        }
        self
    }

    /// Sets how `mod name` finds modules.
    pub fn provider(mut self, provider: impl ModuleProvider + 'static) -> Builder {
        self.provider = Some(Box::new(provider));
        self
    }

    /// Adds (or overrides) a global visible to every script.
    pub fn global(mut self, name: &str, value: Value) -> Builder {
        self.globals.push((name.into(), value));
        self
    }

    /// Registers a native module: `mod name` gives `exports` (usually a map
    /// of functions built with [`Library`](crate::Library)) without asking the provider.
    pub fn module(mut self, name: &str, exports: Value) -> Builder {
        self.modules.push((name.into(), exports));
        self
    }

    /// Where `print` and `println` write (standard output by default).
    pub fn output(mut self, output: impl Write + 'static) -> Builder {
        self.output = Some(Box::new(output));
        self
    }

    /// Seeds the random number generator, for reproducible scripts.
    pub fn seed(mut self, seed: u64) -> Builder {
        self.seed = Some(seed);
        self
    }

    /// Creates the runtime.
    pub fn build(self) -> Runtime {
        let denied = self.denied;
        let library = stdlib::prelude(&|cap| !denied.contains(&cap));
        let mut prelude = Vec::new();
        let mut names = PreludeNames::default();
        let put_overridden = self.globals.iter().any(|(name, _)| &**name == "put");
        for (name, value) in library.into_entries().chain(self.globals) {
            match names.get(&name) {
                Some(i) => prelude[i as usize] = value,
                None => {
                    names.insert(name, prelude.len() as u32);
                    prelude.push(value);
                }
            }
        }
        names.set_builtin_put(if put_overridden {
            None
        } else {
            names.get("put")
        });
        let output = self.output.unwrap_or_else(|| Box::new(std::io::stdout()));
        let rng = SmallRng::seed_from_u64(self.seed.unwrap_or_else(random_seed));
        let mut vm = Vm::new(self.limits, prelude, names, output, rng);
        vm.provider = self.provider;
        vm.native_modules = self.modules.into_iter().collect();
        Runtime { vm, session: None }
    }
}

/// A dog4 interpreter instance.
///
/// It owns loaded modules (each runs once), the prelude and the sandbox
/// configuration. Every call (`run`, `import`, `call`, `eval`) gets a fresh fuel
/// and time budget. A runtime uses reference counting internally and is not
/// `Send`; create one per thread.
///
/// ```
/// let mut rt = dog4::Runtime::new();
/// let out = rt.run("main", "x = 20; put ($x + 1) more").unwrap();
/// assert_eq!(out.to_string(), "21 more");
/// ```
pub struct Runtime {
    vm: Vm,
    session: Option<Rc<ModuleEnv>>,
}

impl Default for Runtime {
    fn default() -> Runtime {
        Runtime::new()
    }
}

impl Runtime {
    /// A runtime with the default (trusted) configuration.
    pub fn new() -> Runtime {
        Builder::new().build()
    }

    /// Starts configuring a runtime.
    pub fn builder() -> Builder {
        Builder::new()
    }

    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        let _ = self.vm.out.flush();
        result.map_err(|e: Error| e.resolve(&self.vm.sources))
    }

    /// Runs `source` as an entry script named `name` and returns what it outputs.
    /// Relative `mod` paths inside it resolve against `name`.
    pub fn run(&mut self, name: &str, source: &str) -> Result<Value> {
        self.vm.reset_meter();
        let result = self.vm.run_source(name, source);
        self.finish(result)
    }

    /// Loads a module through the provider (running it the first time) and
    /// returns its map.
    pub fn import(&mut self, name: &str) -> Result<Value> {
        self.vm.reset_meter();
        let result = self.vm.import(name);
        self.finish(result)
    }

    /// Calls a function value (e.g. one taken from a module map).
    pub fn call(&mut self, function: &Value, args: Vec<Value>) -> Result<Value> {
        self.vm.reset_meter();
        let result = self.vm.call(function, args);
        self.finish(result)
    }

    /// Runs code in a persistent session: globals defined by one call are
    /// visible to the next (this is what the REPL uses).
    pub fn eval(&mut self, code: &str) -> Result<Value> {
        self.vm.reset_meter();
        let env = match &self.session {
            Some(env) => env.clone(),
            None => {
                let env = self.vm.new_env("<repl>".into(), ModuleScope::default());
                self.session = Some(env.clone());
                env
            }
        };
        let result = self.vm.eval_in(&env, "<repl>", code);
        self.finish(result)
    }

    /// A handle that can stop the running script from another thread.
    pub fn interrupt_handle(&self) -> Interrupt {
        self.vm.interrupt.clone()
    }

    /// Compiles `source` and returns a bytecode listing.
    pub fn disassemble(&mut self, name: &str, source: &str) -> Result<String> {
        let id = self.vm.sources.add(name, source);
        let result = parse(source, id, self.vm.limits.max_nesting).and_then(|block| {
            let mut scope = ModuleScope::default();
            compile_module(&block, &self.vm.prelude_names, &mut scope, id)
        });
        self.finish(result.map(|proto| disassemble(&proto)))
    }
}

/// A seed from the standard library's per-process hash randomness. This avoids
/// a dependency on OS entropy APIs, so dog4 also builds for WebAssembly
/// (where hosts should pass their own seed with [`Builder::seed`]).
fn random_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_usize(&hasher as *const _ as usize);
    hasher.finish()
}

/// An in-memory output sink, cheap to clone, for capturing `print` output.
///
/// ```
/// let out = dog4::OutputBuffer::default();
/// let mut rt = dog4::Runtime::builder().output(out.clone()).build();
/// rt.run("main", "println hello").unwrap();
/// assert_eq!(out.take(), "hello\n");
/// ```
#[derive(Clone, Default)]
pub struct OutputBuffer(Rc<RefCell<Vec<u8>>>);

impl OutputBuffer {
    /// Takes everything written so far.
    pub fn take(&self) -> String {
        let bytes = std::mem::take(&mut *self.0.borrow_mut());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl Write for OutputBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
