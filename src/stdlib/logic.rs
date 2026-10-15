//! Comparisons and boolean logic (dog3 names). The comparisons are numeric
//! and convert text, so `eq 1 1.0` is `true` and `eq a b` is an error.

use std::cmp::Ordering;

use super::Library;
use crate::error::Result;
use crate::value::{Value, compare};
use crate::vm::Vm;

fn comparison(test: fn(Ordering) -> bool) -> impl Fn(&mut Vm, &mut [Value]) -> Result<Value> {
    move |_, args| Ok(Value::Bool(compare(&args[0], &args[1])?.is_some_and(test)))
}

pub(super) fn install(lib: &mut Library) {
    lib.def("true", "", |_, _| Ok(Value::Bool(true)));
    lib.def("false", "", |_, _| Ok(Value::Bool(false)));
    lib.def("eq", "a: num, b: num", comparison(Ordering::is_eq));
    lib.def("neq", "a: num, b: num", |_, args| {
        Ok(Value::Bool(
            !compare(&args[0], &args[1])?.is_some_and(Ordering::is_eq),
        ))
    });
    lib.def("lt", "a: num, b: num", comparison(Ordering::is_lt));
    lib.def("gt", "a: num, b: num", comparison(Ordering::is_gt));
    lib.def("leq", "a: num, b: num", comparison(Ordering::is_le));
    lib.def("geq", "a: num, b: num", comparison(Ordering::is_ge));
    lib.def("like", "a, b", |_, args| {
        Ok(Value::Bool(args[0].to_string() == args[1].to_string()))
    });
    lib.def("and", "a, b", |_, args| {
        Ok(Value::Bool(args[0].truthy() && args[1].truthy()))
    });
    lib.def("or", "a, b", |_, args| {
        Ok(Value::Bool(args[0].truthy() || args[1].truthy()))
    });
    lib.def("not", "a", |_, args| Ok(Value::Bool(!args[0].truthy())));
}
