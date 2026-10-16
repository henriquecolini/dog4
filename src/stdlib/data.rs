//! Editing arrays and maps. Like every builtin, these return a new value: write
//! `list = { insert $list 0 x }` to change a variable.

use rustc_hash::FxHashSet;

use super::{Library, int, take_items};
use crate::error::{Error, Result};
use crate::value::{Map, Value, key_text, write_repr};
use crate::vm::Vm;

/// An index into an array of `len` items for `insert` / `remove_at`: negative
/// indices count from the end, and `insert` may also use `len` (to append).
fn position(value: &Value, len: usize, inserting: bool, what: &str) -> Result<usize> {
    let i = int(value, "an index")?;
    let end = len as i64 + i64::from(inserting);
    let resolved = if i < 0 { i + len as i64 } else { i };
    if (0..end).contains(&resolved) {
        Ok(resolved as usize)
    } else {
        Err(Error::value_error(format!(
            "can't {what} at index {i} of an array of {len}"
        )))
    }
}

fn map_arg(value: &mut Value) -> Map {
    match std::mem::take(value) {
        Value::Map(map) => map.into_map(),
        _ => Map::default(),
    }
}

pub(super) fn install(lib: &mut Library) {
    // Maps.
    lib.def("remove", "map: map, %keys", |_, args| {
        let mut map = map_arg(&mut args[0]);
        for key in &args[1..] {
            map.shift_remove(&*key_text(key));
        }
        Ok(Value::map(map))
    });
    lib.def("has", "map: map, key", |_, args| {
        let Value::Map(map) = &args[0] else {
            return Ok(Value::Bool(false));
        };
        Ok(Value::Bool(map.contains_key(&*key_text(&args[1]))))
    });
    lib.def("entries", "map: map", |vm, args| {
        let map = map_arg(&mut args[0]);
        vm.meter().tick(map.len() as u64)?;
        Ok(Value::arr(
            map.into_iter()
                .map(|(k, v)| Value::arr(vec![Value::str(&*k), v]))
                .collect(),
        ))
    });
    lib.def("from_entries", "pairs: arr", |vm, args| {
        let mut map = Map::default();
        for pair in take_items(&mut args[0]) {
            vm.meter().tick(1)?;
            let Value::Arr(pair) = pair else {
                return Err(Error::type_error(format!(
                    "`from_entries` needs [key value] pairs, got {}",
                    pair.describe()
                )));
            };
            let [key, value] = &pair[..] else {
                return Err(Error::value_error(format!(
                    "`from_entries` needs [key value] pairs, got one of {} items",
                    pair.len()
                )));
            };
            map.insert(key_text(key), value.clone());
        }
        Ok(Value::map(map))
    });

    // Arrays.
    lib.def("insert", "items: arr, index: num, value", |vm, args| {
        let mut items = take_items(&mut args[0]);
        let at = position(&args[1], items.len(), true, "insert")?;
        vm.meter().coll_len(items.len() + 1)?;
        vm.meter().tick(items.len() as u64 / 8)?;
        items.insert(at, std::mem::take(&mut args[2]));
        Ok(Value::arr(items))
    });
    lib.def("remove_at", "items: arr, index: num", |vm, args| {
        let mut items = take_items(&mut args[0]);
        let at = position(&args[1], items.len(), false, "remove")?;
        vm.meter().tick(items.len() as u64 / 8)?;
        items.remove(at);
        Ok(Value::arr(items))
    });
    lib.def("flatten", "items: arr", |vm, args| {
        let mut out = Vec::new();
        for item in take_items(&mut args[0]) {
            vm.meter().tick(1)?;
            match item {
                Value::Arr(inner) => {
                    vm.meter().coll_len(out.len() + inner.len())?;
                    out.extend(inner.into_vec());
                }
                other => out.push(other),
            }
        }
        Ok(Value::arr(out))
    });
    lib.def("unique", "items: arr", |vm, args| {
        let items = take_items(&mut args[0]);
        vm.meter().tick(items.len() as u64)?;
        Ok(Value::arr(unique(vm, items)?))
    });
    lib.def("zip", "left: arr, right: arr", |vm, args| {
        let right = take_items(&mut args[1]);
        vm.meter().tick(right.len() as u64)?;
        Ok(Value::arr(
            take_items(&mut args[0])
                .into_iter()
                .zip(right)
                .map(|(a, b)| Value::arr(vec![a, b]))
                .collect(),
        ))
    });
    lib.def("chunk", "items: arr, size: num", |vm, args| {
        let size = int(&args[1], "a chunk size")?;
        let size = usize::try_from(size)
            .ok()
            .filter(|&s| s > 0)
            .ok_or_else(|| {
                Error::value_error(format!("a chunk size must be positive, not {size}"))
            })?;
        let items = take_items(&mut args[0]);
        vm.meter().tick(items.len() as u64)?;
        Ok(Value::arr(
            items.chunks(size).map(|c| Value::arr(c.to_vec())).collect(),
        ))
    });
}

/// The items without repeats, keeping the first of each. Values are compared
/// by their literal form, except functions, which are compared by identity.
fn unique(vm: &mut Vm, items: Vec<Value>) -> Result<Vec<Value>> {
    let mut seen = FxHashSet::default();
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        if matches!(item, Value::Func(_)) {
            if !out.contains(&item) {
                out.push(item);
            }
            continue;
        }
        let mut key = String::new();
        write_repr(&item, &mut key, 0);
        vm.meter().tick(key.len() as u64 / 16)?;
        if seen.insert(key) {
            out.push(item);
        }
    }
    Ok(out)
}
