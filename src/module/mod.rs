//! Module providers: how `use "name"` finds a module.
//!
//! The runtime doesn't know about files. It asks a [`ModuleProvider`] to turn a
//! name into a canonical id, then to load that id. Ids are used for caching (a
//! module runs once per runtime) and for resolving relative names.

use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::value::Value;

/// What a provider returns for a module id.
pub enum ModuleSource {
    /// dog4 source code, compiled and run on first use.
    Script(String),
    /// A ready-made map (usually of native functions).
    Native(Value),
}

/// Why a provider couldn't produce a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The provider doesn't know this module. [`ChainProvider`] moves on to the next one.
    NotFound,
    /// The module exists but couldn't be loaded.
    Failed(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::NotFound => f.write_str("not found"),
            ProviderError::Failed(message) => f.write_str(message),
        }
    }
}

/// Supplies modules to a runtime.
///
/// Implement this to load modules from anywhere: a database, an HTTP service,
/// a virtual filesystem. Built in: [`FsProvider`], [`MemoryProvider`], [`ChainProvider`].
pub trait ModuleProvider {
    /// Turns the name written in `use` into a canonical id. `from` is the id of
    /// the importing module (`None` for code run directly by the host).
    ///
    /// The default treats `./x` and `../x` as relative to `from` and everything
    /// else as already canonical; see [`resolve_relative`].
    fn resolve(&self, name: &str, from: Option<&str>) -> Result<String, ProviderError> {
        resolve_relative(name, from)
    }

    /// Loads the module with a canonical id.
    fn load(&self, id: &str) -> Result<ModuleSource, ProviderError>;
}

/// Resolves `./name` and `../name` against the directory of `from`.
/// Other names are returned normalised. Ids never start with `/` and never
/// escape the root with `..`.
pub fn resolve_relative(name: &str, from: Option<&str>) -> Result<String, ProviderError> {
    let relative = name.starts_with("./") || name.starts_with("../");
    let mut parts: Vec<&str> = Vec::new();
    if relative {
        if let Some(from) = from {
            parts.extend(from.split('/'));
            parts.pop();
        }
    }
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(ProviderError::Failed(format!(
                        "`{name}` points outside the module root"
                    )));
                }
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(ProviderError::NotFound);
    }
    let id = parts.join("/");
    Ok(id.strip_suffix(".dog").map(str::to_string).unwrap_or(id))
}

/// Loads `id` from `<root>/<id>.dog`.
#[derive(Debug, Clone)]
pub struct FsProvider {
    root: PathBuf,
}

impl FsProvider {
    /// Serves modules from files under `root`.
    pub fn new(root: impl Into<PathBuf>) -> FsProvider {
        FsProvider { root: root.into() }
    }

    /// The file a module id maps to.
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        let relative = Path::new(id);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return None;
        }
        Some(self.root.join(format!("{id}.dog")))
    }
}

impl ModuleProvider for FsProvider {
    fn load(&self, id: &str) -> Result<ModuleSource, ProviderError> {
        let path = self.path_of(id).ok_or(ProviderError::NotFound)?;
        match std::fs::read_to_string(&path) {
            Ok(source) => Ok(ModuleSource::Script(source)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ProviderError::NotFound),
            Err(e) => Err(ProviderError::Failed(format!("{}: {e}", path.display()))),
        }
    }
}

/// Modules held in memory: handy for tests, bots and sandboxes.
#[derive(Default)]
pub struct MemoryProvider {
    modules: HashMap<String, MemoryModule>,
}

enum MemoryModule {
    Script(String),
    Native(Value),
}

impl MemoryProvider {
    /// An empty provider.
    pub fn new() -> MemoryProvider {
        MemoryProvider::default()
    }

    /// Adds a script module.
    pub fn with(mut self, id: &str, source: &str) -> MemoryProvider {
        self.insert(id, source);
        self
    }

    /// Adds or replaces a script module.
    pub fn insert(&mut self, id: &str, source: &str) {
        self.modules
            .insert(id.to_string(), MemoryModule::Script(source.to_string()));
    }

    /// Adds or replaces a native module (a map value).
    pub fn insert_native(&mut self, id: &str, exports: Value) {
        self.modules
            .insert(id.to_string(), MemoryModule::Native(exports));
    }
}

impl ModuleProvider for MemoryProvider {
    fn load(&self, id: &str) -> Result<ModuleSource, ProviderError> {
        match self.modules.get(id) {
            Some(MemoryModule::Script(source)) => Ok(ModuleSource::Script(source.clone())),
            Some(MemoryModule::Native(value)) => Ok(ModuleSource::Native(value.clone())),
            None => Err(ProviderError::NotFound),
        }
    }
}

/// Tries several providers in order; the first that doesn't say
/// [`ProviderError::NotFound`] wins. Ids are prefixed with the provider's
/// position so equal names from different providers don't collide.
#[derive(Default)]
pub struct ChainProvider {
    providers: Vec<Box<dyn ModuleProvider>>,
}

impl ChainProvider {
    /// An empty chain.
    pub fn new() -> ChainProvider {
        ChainProvider::default()
    }

    /// Appends a provider.
    pub fn with(mut self, provider: impl ModuleProvider + 'static) -> ChainProvider {
        self.providers.push(Box::new(provider));
        self
    }
}

impl ModuleProvider for ChainProvider {
    fn resolve(&self, name: &str, from: Option<&str>) -> Result<String, ProviderError> {
        let inner_from = from.and_then(|f| f.split_once(':')).map(|(_, id)| id);
        for (i, provider) in self.providers.iter().enumerate() {
            let id = match provider.resolve(name, inner_from) {
                Ok(id) => id,
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            match provider.load(&id) {
                Ok(_) => return Ok(format!("{i}:{id}")),
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(ProviderError::NotFound)
    }

    fn load(&self, id: &str) -> Result<ModuleSource, ProviderError> {
        let (index, inner) = id.split_once(':').ok_or(ProviderError::NotFound)?;
        let index: usize = index.parse().map_err(|_| ProviderError::NotFound)?;
        self.providers
            .get(index)
            .ok_or(ProviderError::NotFound)?
            .load(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_resolution() {
        assert_eq!(resolve_relative("a/b", None).unwrap(), "a/b");
        assert_eq!(resolve_relative("a/b.dog", None).unwrap(), "a/b");
        assert_eq!(resolve_relative("./c", Some("a/b")).unwrap(), "a/c");
        assert_eq!(resolve_relative("../c", Some("a/b/x")).unwrap(), "a/c");
        assert_eq!(resolve_relative("./c", None).unwrap(), "c");
        assert!(resolve_relative("../c", Some("a")).is_err());
        assert!(resolve_relative("../../etc/passwd", Some("a/b")).is_err());
        assert_eq!(resolve_relative("", None), Err(ProviderError::NotFound));
    }

    #[test]
    fn fs_paths_stay_inside_root() {
        let fs = FsProvider::new("/srv/mods");
        assert_eq!(
            fs.path_of("a/b").unwrap(),
            PathBuf::from("/srv/mods/a/b.dog")
        );
        assert!(fs.path_of("../x").is_none());
        assert!(fs.path_of("/etc/passwd").is_none());
    }

    #[test]
    fn chain_falls_through() {
        let chain = ChainProvider::new()
            .with(MemoryProvider::new().with("a", "x = 1"))
            .with(MemoryProvider::new().with("a", "x = 2").with("b", "y = 1"));
        let a = chain.resolve("a", None).unwrap();
        let b = chain.resolve("b", None).unwrap();
        assert_eq!(a, "0:a");
        assert_eq!(b, "1:b");
        assert!(matches!(chain.load(&b), Ok(ModuleSource::Script(s)) if s == "y = 1"));
        assert_eq!(chain.resolve("c", None), Err(ProviderError::NotFound));
    }
}
