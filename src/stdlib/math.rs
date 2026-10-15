//! Arithmetic builtins (dog3 names) and randomness.

use rand::Rng;
use rand::seq::SliceRandom;

use super::{Library, int, items, num, take_items};
use crate::error::{Error, Result};
use crate::value::{self, Value};
use crate::vm::Vm;

/// Folds `first` and every number in `rest` with `op`, left to right.
fn fold(args: &[Value], op: fn(&Value, &Value) -> Result<Value>) -> Result<Value> {
    let mut acc = args[0].clone();
    for x in items(&args[1]) {
        acc = op(&acc, x)?;
    }
    Ok(acc)
}

fn add(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let mut acc = std::mem::take(&mut args[0]);
    for x in take_items(&mut args[1]) {
        value::add_assign(&mut acc, x, vm.meter())?;
    }
    Ok(acc)
}

fn unary(f: fn(f64) -> f64) -> impl Fn(&mut Vm, &mut [Value]) -> Result<Value> {
    move |_, args| Ok(Value::num(f(num(&args[0]))))
}

pub(super) fn install(lib: &mut Library) {
    lib.def("add", "first: num, %rest: num", add);
    lib.def("sub", "first: num, %rest: num", |_, args| {
        fold(args, value::sub)
    });
    lib.def("mul", "first: num, %rest: num", |vm, args| {
        let mut acc = args[0].clone();
        for x in items(&args[1]) {
            acc = value::mul(&acc, x, vm.meter())?;
        }
        Ok(acc)
    });
    lib.def("div", "first: num, %rest: num", |_, args| {
        fold(args, value::div)
    });
    lib.def("max", "first: num, %rest: num", |_, args| {
        let best = items(&args[1])
            .iter()
            .map(num)
            .fold(num(&args[0]), f64::max);
        Ok(Value::num(best))
    });
    lib.def("min", "first: num, %rest: num", |_, args| {
        let best = items(&args[1])
            .iter()
            .map(num)
            .fold(num(&args[0]), f64::min);
        Ok(Value::num(best))
    });
    lib.def("floor", "x: num", unary(f64::floor));
    lib.def("ceil", "x: num", unary(f64::ceil));
    lib.def("round", "x: num", unary(f64::round));
    lib.def("abs", "x: num", unary(f64::abs));
    lib.def("sqrt", "x: num", unary(f64::sqrt));
    lib.def("pow", "base: num, exponent: num", |_, args| {
        if let (Value::Int(b), Value::Int(e)) = (&args[0], &args[1]) {
            if let Some(r) = u32::try_from(*e).ok().and_then(|e| b.checked_pow(e)) {
                return Ok(Value::Int(r));
            }
        }
        Ok(Value::num(num(&args[0]).powf(num(&args[1]))))
    });
}

fn random_between(vm: &mut Vm, lo: i64, hi: i64) -> Result<Value> {
    if lo >= hi {
        return Err(Error::value_error(format!(
            "`random` needs min < max, got {lo} and {hi}"
        )));
    }
    Ok(Value::Int(vm.rng().gen_range(lo..hi)))
}

pub(super) fn install_random(lib: &mut Library) {
    lib.def("random", "max: num", |vm, args| {
        let hi = int(&args[0], "`random`'s max")?;
        random_between(vm, 0, hi)
    });
    lib.def("random", "min: num, max: num", |vm, args| {
        let lo = int(&args[0], "`random`'s min")?;
        let hi = int(&args[1], "`random`'s max")?;
        random_between(vm, lo, hi)
    });
    lib.def("shuffle", "items: arr", |vm, args| {
        let mut items = take_items(&mut args[0]);
        items.shuffle(vm.rng());
        Ok(Value::arr(items))
    });
    lib.def("pick", "items: arr", |vm, args| {
        let items = items(&args[0]);
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let i = vm.rng().gen_range(0..items.len());
        Ok(items[i].clone())
    });
}
