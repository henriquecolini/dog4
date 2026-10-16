//! Higher-order builtins: they take callbacks.

use super::iter::sort_values;
use super::{Library, take_items};
use crate::error::{Error, Result};
use crate::value::{Map, Value, compare, key_text};
use crate::vm::Vm;

/// The item whose key (`f` of the item) is smallest (`want` = `Less`) or largest.
fn extreme_by(vm: &mut Vm, args: &mut [Value], want: std::cmp::Ordering) -> Result<Value> {
    let f = std::mem::take(&mut args[1]);
    let mut best: Option<(Value, Value)> = None;
    for item in take_items(&mut args[0]) {
        let key = vm.call(&f, vec![item.clone()])?;
        let better = match &best {
            None => true,
            Some((best_key, _)) => match compare(&key, best_key)? {
                Some(ordering) => ordering == want,
                None => {
                    return Err(Error::type_error(format!(
                        "can't compare {} with {}",
                        key.describe(),
                        best_key.describe()
                    )));
                }
            },
        };
        if better {
            best = Some((key, item));
        }
    }
    Ok(best.map_or(Value::Null, |(_, item)| item))
}

pub(super) fn install(lib: &mut Library) {
    lib.def("map", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        let mut out = Vec::new();
        for item in take_items(&mut args[0]) {
            out.push(vm.call(&f, vec![item])?);
        }
        Ok(Value::arr(out))
    });
    lib.def("filter", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        let mut out = Vec::new();
        for item in take_items(&mut args[0]) {
            if vm.call(&f, vec![item.clone()])?.truthy() {
                out.push(item);
            }
        }
        Ok(Value::arr(out))
    });
    lib.def("reduce", "items: arr, initial, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[2]);
        let mut acc = std::mem::take(&mut args[1]);
        for item in take_items(&mut args[0]) {
            acc = vm.call(&f, vec![acc, item])?;
        }
        Ok(acc)
    });
    lib.def("each", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        for item in take_items(&mut args[0]) {
            vm.call(&f, vec![item])?;
        }
        Ok(Value::Null)
    });
    lib.def("sort_by", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        let items = take_items(&mut args[0]);
        let mut keyed = Vec::with_capacity(items.len());
        for item in items {
            let key = vm.call(&f, vec![item.clone()])?;
            keyed.push(Value::arr(vec![key, item]));
        }
        sort_values(&mut keyed, |pair| match pair {
            Value::Arr(p) => p[0].clone(),
            _ => Value::Null,
        })?;
        Ok(Value::arr(
            keyed
                .into_iter()
                .map(|pair| match pair {
                    Value::Arr(p) => p.into_vec().pop().unwrap_or_default(),
                    _ => Value::Null,
                })
                .collect(),
        ))
    });
    lib.def("any", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        for item in take_items(&mut args[0]) {
            if vm.call(&f, vec![item])?.truthy() {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    });
    lib.def("all", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        for item in take_items(&mut args[0]) {
            if !vm.call(&f, vec![item])?.truthy() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    });
    lib.def("find", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        for item in take_items(&mut args[0]) {
            if vm.call(&f, vec![item.clone()])?.truthy() {
                return Ok(item);
            }
        }
        Ok(Value::Null)
    });
    lib.def("find_index", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        for (i, item) in take_items(&mut args[0]).into_iter().enumerate() {
            if vm.call(&f, vec![item])?.truthy() {
                return Ok(Value::Int(i as i64));
            }
        }
        Ok(Value::Null)
    });
    lib.def("flat_map", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        let mut out = Vec::new();
        for item in take_items(&mut args[0]) {
            match vm.call(&f, vec![item])? {
                Value::Arr(more) => {
                    vm.meter().coll_len(out.len() + more.len())?;
                    out.extend(more.into_vec());
                }
                other => out.push(other),
            }
        }
        Ok(Value::arr(out))
    });
    // A map from each key (`f` of an item, as text) to the items with that key.
    lib.def("group_by", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        let mut groups: Map = Map::default();
        for item in take_items(&mut args[0]) {
            let key = key_text(&vm.call(&f, vec![item.clone()])?);
            let group = groups.entry(key).or_insert_with(|| Value::arr(Vec::new()));
            if let Value::Arr(group) = group {
                group.make_mut().push(item);
            }
        }
        Ok(Value::map(groups))
    });
    lib.def("count_by", "items: arr, f: fn", |vm, args| {
        let f = std::mem::take(&mut args[1]);
        let mut counts: Map = Map::default();
        for item in take_items(&mut args[0]) {
            let key = key_text(&vm.call(&f, vec![item])?);
            if let Value::Int(n) = counts.entry(key).or_insert(Value::Int(0)) {
                *n += 1;
            }
        }
        Ok(Value::map(counts))
    });
    lib.def("min_by", "items: arr, f: fn", |vm, args| {
        extreme_by(vm, args, std::cmp::Ordering::Less)
    });
    lib.def("max_by", "items: arr, f: fn", |vm, args| {
        extreme_by(vm, args, std::cmp::Ordering::Greater)
    });
}
