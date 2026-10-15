//! Higher-order builtins: they take callbacks.

use super::iter::sort_values;
use super::{Library, take_items};
use crate::value::Value;

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
}
