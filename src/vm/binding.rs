//! Shared variables: cells (captured locals), references, and module globals.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compile::ModuleScope;
use crate::error::{Error, Result};
use crate::limits::Meter;
use crate::value::{Map, UpdateOp, Value, get_path, update_path};

/// A boxed variable shared between a frame and the closures that captured it.
pub type Cell = Rc<RefCell<Value>>;

/// How a frame cell or upvalue reaches its value.
#[derive(Clone)]
pub enum Binding {
    /// An ordinary shared variable.
    Cell(Cell),
    /// An alias of someone else's variable (a `&` parameter or `let r = &x`).
    Ref(Rc<RefData>),
}

/// The variable a reference points into.
#[derive(Clone)]
pub enum RefTarget {
    /// A local variable (boxed into a cell).
    Cell(Cell),
    /// A module global.
    Global(Rc<ModuleEnv>, u32),
}

/// A reference: a variable plus a path inside it (`&user.tags[0]`).
pub struct RefData {
    target: RefTarget,
    path: Box<[Value]>,
}

impl RefTarget {
    fn with_mut<R>(&self, f: impl FnOnce(&mut Value) -> Result<R>) -> Result<R> {
        match self {
            RefTarget::Cell(cell) => f(&mut cell.borrow_mut()),
            RefTarget::Global(env, slot) => {
                let mut globals = env.globals.borrow_mut();
                match globals.get_mut(*slot as usize) {
                    Some(Some(value)) => f(value),
                    _ => Err(env.uninitialized(*slot)),
                }
            }
        }
    }

    fn read(&self) -> Result<Value> {
        match self {
            RefTarget::Cell(cell) => Ok(cell.borrow().clone()),
            RefTarget::Global(env, slot) => env.get(*slot),
        }
    }
}

impl RefData {
    /// A reference to `target` at `path`.
    pub fn new(target: RefTarget, path: Vec<Value>) -> RefData {
        RefData {
            target,
            path: path.into_boxed_slice(),
        }
    }

    /// The current value at the referenced place.
    pub fn read(&self) -> Result<Value> {
        let root = self.target.read()?;
        if self.path.is_empty() {
            Ok(root)
        } else {
            get_path(&root, &self.path)
        }
    }

    /// Applies an update at `keys` below the referenced place.
    pub fn update(
        &self,
        keys: &[Value],
        op: UpdateOp,
        value: Value,
        meter: &mut Meter,
    ) -> Result<()> {
        if self.path.is_empty() {
            return self
                .target
                .with_mut(|root| update_path(root, keys, op, value, meter));
        }
        let full: Vec<Value> = self.path.iter().chain(keys).cloned().collect();
        self.target
            .with_mut(|root| update_path(root, &full, op, value, meter))
    }

    /// A reference further down: `&r.field` where `r` is itself a reference.
    pub fn extend(&self, keys: Vec<Value>) -> RefData {
        let path = self.path.iter().cloned().chain(keys).collect();
        RefData::new(self.target.clone(), path)
    }
}

impl Binding {
    /// The current value.
    pub fn read(&self) -> Result<Value> {
        match self {
            Binding::Cell(cell) => Ok(cell.borrow().clone()),
            Binding::Ref(r) => r.read(),
        }
    }

    /// Overwrites the value.
    pub fn write(&self, value: Value, meter: &mut Meter) -> Result<()> {
        match self {
            Binding::Cell(cell) => {
                let old = std::mem::replace(&mut *cell.borrow_mut(), value);
                drop(old);
                Ok(())
            }
            Binding::Ref(r) => r.update(&[], UpdateOp::Set, value, meter),
        }
    }

    /// Updates the value (or a path inside it) in place.
    pub fn update(
        &self,
        keys: &[Value],
        op: UpdateOp,
        value: Value,
        meter: &mut Meter,
    ) -> Result<()> {
        match self {
            Binding::Cell(cell) => update_path(&mut cell.borrow_mut(), keys, op, value, meter),
            Binding::Ref(r) => r.update(keys, op, value, meter),
        }
    }

    /// A reference to this variable at `keys`.
    pub fn make_ref(&self, keys: Vec<Value>) -> RefData {
        match self {
            Binding::Cell(cell) => RefData::new(RefTarget::Cell(cell.clone()), keys),
            Binding::Ref(r) => r.extend(keys),
        }
    }
}

/// The live state of one module: its global names and values.
pub struct ModuleEnv {
    /// The module id (or a display name for entry scripts).
    pub name: Rc<str>,
    /// Global names (grows when `eval` declares new ones).
    pub scope: RefCell<ModuleScope>,
    /// Global values, `None` until first assigned.
    pub globals: RefCell<Vec<Option<Value>>>,
}

impl ModuleEnv {
    /// A module with the given globals, all uninitialized.
    pub fn new(name: Rc<str>, scope: ModuleScope) -> ModuleEnv {
        let globals = vec![None; scope.names().len()];
        ModuleEnv {
            name,
            scope: RefCell::new(scope),
            globals: RefCell::new(globals),
        }
    }

    /// Makes room for globals added to the scope since creation.
    pub fn sync(&self) {
        let len = self.scope.borrow().names().len();
        let mut globals = self.globals.borrow_mut();
        if globals.len() < len {
            globals.resize(len, None);
        }
    }

    /// Reads a global.
    pub fn get(&self, slot: u32) -> Result<Value> {
        match self.globals.borrow().get(slot as usize) {
            Some(Some(value)) => Ok(value.clone()),
            _ => Err(self.uninitialized(slot)),
        }
    }

    /// Writes a global.
    pub fn set(&self, slot: u32, value: Value) {
        let old = self.globals.borrow_mut()[slot as usize].replace(value);
        drop(old);
    }

    /// Updates a global in place.
    pub fn update(
        &self,
        slot: u32,
        keys: &[Value],
        op: UpdateOp,
        value: Value,
        meter: &mut Meter,
    ) -> Result<()> {
        let mut globals = self.globals.borrow_mut();
        // `count += 1` on whole numbers: skip the general path.
        if keys.is_empty() && op == UpdateOp::Add {
            if let (Some(Some(Value::Int(x))), Value::Int(y)) =
                (globals.get_mut(slot as usize), &value)
            {
                if let Some(r) = x.checked_add(*y) {
                    *x = r;
                    return Ok(());
                }
            }
        }
        match globals.get_mut(slot as usize) {
            Some(Some(current)) => update_path(current, keys, op, value, meter),
            _ => {
                drop(globals);
                Err(self.uninitialized(slot))
            }
        }
    }

    /// Looks a name up the way the module's top level sees it: its globals,
    /// then its glob imports.
    pub fn lookup(&self, name: &str) -> Option<Value> {
        let scope = self.scope.borrow();
        if let Some(slot) = scope.get(name) {
            return self.get(slot).ok();
        }
        let globals = self.globals.borrow();
        scope
            .globs()
            .iter()
            .rev()
            .find_map(|&g| match globals.get(g as usize) {
                Some(Some(Value::Map(map))) => map.get(name).cloned(),
                _ => None,
            })
    }

    fn uninitialized(&self, slot: u32) -> Error {
        let scope = self.scope.borrow();
        let name = scope.names().get(slot as usize).map_or("?", |n| &**n);
        Error::name_error(format!("`{name}` is used before it has a value"))
    }

    /// The module's map: every initialized `pub` global, then the members of
    /// its `pub use` glob imports that no `pub` name or newer glob provides.
    pub fn exports(&self) -> Value {
        let scope = self.scope.borrow();
        let globals = self.globals.borrow();
        let globs = scope.globs();
        let mut map: Map = scope
            .names()
            .iter()
            .zip(globals.iter())
            .enumerate()
            .filter(|(slot, _)| scope.is_public(*slot as u32) && !globs.contains(&(*slot as u32)))
            .filter_map(|(_, (name, value))| Some((name.clone(), value.clone()?)))
            .collect();
        for &g in globs.iter().rev().filter(|&&g| scope.is_public(g)) {
            if let Some(Some(Value::Map(glob))) = globals.get(g as usize) {
                for (name, value) in glob.iter() {
                    if !map.contains_key(name) {
                        map.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        Value::map(map)
    }

    /// Clears every global, breaking reference cycles through module functions.
    pub fn clear(&self) {
        let old = std::mem::take(&mut *self.globals.borrow_mut());
        drop(old);
    }
}
