//! Arithmetic builtins (dog3 names) and randomness.

use rand::Rng;
use rand::seq::SliceRandom;

use super::{Library, checked_str, int, items, num, take_items, text};
use crate::error::{Error, Result};
use crate::value::{self, Type, Value, coerce};
use crate::vm::Vm;

/// The numbers in an array argument, converting text like typed `num`
/// parameters do.
fn numbers(vm: &mut Vm, value: &mut Value, who: &str) -> Result<Vec<f64>> {
    let items = take_items(value);
    vm.meter().tick(items.len() as u64)?;
    items
        .into_iter()
        .map(|item| match coerce(item, Type::NUM) {
            Ok(n) => Ok(num(&n)),
            Err(item) => Err(Error::type_error(format!(
                "`{who}` needs numbers, got {}",
                item.describe()
            ))),
        })
        .collect()
}

/// A base for `parse_int` / `to_base`: 2 to 36.
fn base(value: &Value) -> Result<u32> {
    let b = int(value, "a base")?;
    u32::try_from(b)
        .ok()
        .filter(|b| (2..=36).contains(b))
        .ok_or_else(|| Error::value_error(format!("a base must be from 2 to 36, not {b}")))
}

/// Rounds `x` to `digits` decimal places (negative: to tens, hundreds, …).
fn round_to(x: f64, digits: i64) -> f64 {
    let digits = digits.clamp(-308, 308) as i32;
    let scale = 10f64.powi(digits.abs());
    if digits >= 0 {
        let scaled = x * scale;
        if scaled.is_finite() {
            scaled.round() / scale
        } else {
            x
        }
    } else {
        (x / scale).round() * scale
    }
}

/// Folds `first` and every number in `rest` with `op`, left to right.
fn fold(args: &[Value], op: fn(&Value, &Value) -> Result<Value>) -> Result<Value> {
    let mut acc = args[0].clone();
    for x in &args[1..] {
        acc = op(&acc, x)?;
    }
    Ok(acc)
}

fn add(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let (first, rest) = args.split_at_mut(1);
    let mut acc = std::mem::take(&mut first[0]);
    for x in rest {
        value::add_assign(&mut acc, std::mem::take(x), vm.meter())?;
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
        for x in &args[1..] {
            acc = value::mul(&acc, x, vm.meter())?;
        }
        Ok(acc)
    });
    lib.def("div", "first: num, %rest: num", |_, args| {
        fold(args, value::div)
    });
    lib.def("max", "first: num, %rest: num", |_, args| {
        let best = args[1..].iter().map(num).fold(num(&args[0]), f64::max);
        Ok(Value::num(best))
    });
    lib.def("min", "first: num, %rest: num", |_, args| {
        let best = args[1..].iter().map(num).fold(num(&args[0]), f64::min);
        Ok(Value::num(best))
    });
    lib.def("floor", "x: num", unary(f64::floor));
    lib.def("ceil", "x: num", unary(f64::ceil));
    lib.def("round", "x: num", unary(f64::round));
    lib.def("abs", "x: num", unary(f64::abs));
    lib.def("sqrt", "x: num", unary(f64::sqrt));
    lib.def("round", "x: num, digits: num", |_, args| {
        let digits = int(&args[1], "a number of digits")?;
        Ok(Value::num(round_to(num(&args[0]), digits)))
    });
    lib.def("trunc", "x: num", unary(f64::trunc));
    lib.def("sign", "x: num", |_, args| {
        let x = num(&args[0]);
        Ok(if x.is_nan() {
            Value::num(x)
        } else if x > 0.0 {
            Value::Int(1)
        } else if x < 0.0 {
            Value::Int(-1)
        } else {
            Value::Int(0)
        })
    });
    lib.def("clamp", "x: num, min: num, max: num", |_, args| {
        let (x, lo, hi) = (num(&args[0]), num(&args[1]), num(&args[2]));
        if lo > hi {
            return Err(Error::value_error(format!(
                "`clamp` needs min <= max, got {} and {}",
                args[1], args[2]
            )));
        }
        Ok(if x < lo {
            args[1].clone()
        } else if x > hi {
            args[2].clone()
        } else {
            args[0].clone()
        })
    });
    lib.def("exp", "x: num", unary(f64::exp));
    lib.def("ln", "x: num", unary(f64::ln));
    lib.def("log", "x: num", unary(f64::log10));
    lib.def("log", "x: num, base: num", |_, args| {
        Ok(Value::num(num(&args[0]).log(num(&args[1]))))
    });
    lib.def("sin", "x: num", unary(f64::sin));
    lib.def("cos", "x: num", unary(f64::cos));
    lib.def("tan", "x: num", unary(f64::tan));
    lib.def("asin", "x: num", unary(f64::asin));
    lib.def("acos", "x: num", unary(f64::acos));
    lib.def("atan", "x: num", unary(f64::atan));
    lib.def("atan2", "y: num, x: num", |_, args| {
        Ok(Value::num(num(&args[0]).atan2(num(&args[1]))))
    });
    lib.def("pi", "", |_, _| Ok(Value::num(std::f64::consts::PI)));

    // Number formatting and bases.
    lib.def("fixed", "x: num, digits: num", |vm, args| {
        let digits = int(&args[1], "a number of digits")?;
        let digits = usize::try_from(digits)
            .ok()
            .filter(|&d| d <= 100)
            .ok_or_else(|| {
                Error::value_error(format!("`fixed` takes 0 to 100 digits, not {digits}"))
            })?;
        checked_str(vm, format!("{:.*}", digits, num(&args[0])))
    });
    lib.def("parse_int", "text: str, base: num", |_, args| {
        let b = base(&args[1])?;
        let digits = text(&args[0]).trim();
        i64::from_str_radix(digits, b).map(Value::Int).map_err(|_| {
            Error::value_error(format!(
                "{} is not a whole number in base {b}",
                args[0].quoted_repr()
            ))
        })
    });
    lib.def("to_base", "x: num, base: num", |_, args| {
        let b = base(&args[1])?;
        let n = int(&args[0], "`to_base`'s number")?;
        let mut digits = Vec::new();
        let mut rest = n.unsigned_abs();
        loop {
            let d = (rest % u64::from(b)) as u32;
            digits.push(char::from_digit(d, b).unwrap_or('?'));
            rest /= u64::from(b);
            if rest == 0 {
                break;
            }
        }
        if n < 0 {
            digits.push('-');
        }
        Ok(Value::str(digits.into_iter().rev().collect::<String>()))
    });

    // Statistics.
    lib.def("product", "items: arr", |vm, args| {
        let mut acc = Value::Int(1);
        for x in take_items(&mut args[0]) {
            vm.meter().tick(1)?;
            acc = value::mul(&acc, &x, vm.meter())?;
        }
        Ok(acc)
    });
    lib.def("avg", "items: arr", |vm, args| {
        let xs = numbers(vm, &mut args[0], "avg")?;
        if xs.is_empty() {
            return Ok(Value::Null);
        }
        Ok(Value::num(xs.iter().sum::<f64>() / xs.len() as f64))
    });
    lib.def("median", "items: arr", |vm, args| {
        let mut xs = numbers(vm, &mut args[0], "median")?;
        if xs.is_empty() {
            return Ok(Value::Null);
        }
        xs.sort_by(f64::total_cmp);
        let mid = xs.len() / 2;
        Ok(Value::num(if xs.len() % 2 == 0 {
            (xs[mid - 1] + xs[mid]) / 2.0
        } else {
            xs[mid]
        }))
    });
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
    lib.def("random_float", "", |vm, _| {
        Ok(Value::num(vm.rng().r#gen::<f64>()))
    });
    lib.def("random_float", "min: num, max: num", |vm, args| {
        let (lo, hi) = (num(&args[0]), num(&args[1]));
        if lo >= hi || !(hi - lo).is_finite() {
            return Err(Error::value_error(format!(
                "`random_float` needs finite min < max, got {} and {}",
                args[0], args[1]
            )));
        }
        Ok(Value::num(vm.rng().gen_range(lo..hi)))
    });
    lib.def("sample", "items: arr, n: num", |vm, args| {
        let n = int(&args[1], "a sample size")?;
        let mut items = take_items(&mut args[0]);
        let n = usize::try_from(n)
            .ok()
            .filter(|&n| n <= items.len())
            .ok_or_else(|| {
                Error::value_error(format!(
                    "can't sample {n} items from an array of {}",
                    items.len()
                ))
            })?;
        vm.meter().tick(n as u64)?;
        let (picked, _) = items.partial_shuffle(vm.rng(), n);
        Ok(Value::arr(picked.to_vec()))
    });
    // A random (version 4) UUID.
    lib.def("uuid", "", |vm, _| {
        let mut bytes: [u8; 16] = vm.rng().r#gen();
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Value::str(format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )))
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
