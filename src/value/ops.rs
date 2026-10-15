//! Operations on values: the fold (`+`), arithmetic, comparison, indexing and
//! in-place updates.

use std::cmp::Ordering;
use std::rc::Rc;

use super::{Map, Value};
use crate::error::{Error, Result};
use crate::limits::Meter;

/// The fold: `acc = acc + rhs`, done in place when `acc` is uniquely owned.
///
/// This single operation is both the `+` operator and how blocks combine what
/// their statements output.
///
/// | left \ right | result |
/// |---|---|
/// | `null + x`, `x + null` | `x` |
/// | num + num | sum |
/// | str + str, arr + arr | concatenation |
/// | map + map | merge (right wins) |
/// | bool + bool | logical AND |
/// | arr + x / x + arr | push / prepend |
/// | str + num/bool, num/bool + str | text concatenation |
pub fn add_assign(acc: &mut Value, rhs: Value, meter: &mut Meter) -> Result<()> {
    match (&mut *acc, rhs) {
        (_, Value::Null) => {}
        (Value::Null, rhs) => *acc = rhs,
        (Value::Int(a), Value::Int(b)) => {
            *acc = match a.checked_add(b) {
                Some(sum) => Value::Int(sum),
                None => Value::num(*a as f64 + b as f64),
            }
        }
        (left @ (Value::Int(_) | Value::Float(_)), right @ (Value::Int(_) | Value::Float(_))) => {
            *left = Value::num(left.as_f64().unwrap_or(0.0) + right.as_f64().unwrap_or(0.0));
        }
        (Value::Str(a), Value::Str(b)) => {
            meter.grow_str(a.len() + b.len(), b.len())?;
            if a.is_empty() {
                *a = b;
            } else {
                Rc::make_mut(a).push_str(&b);
            }
        }
        (Value::Str(a), b @ (Value::Int(_) | Value::Float(_) | Value::Bool(_))) => {
            let text = b.to_string();
            meter.grow_str(a.len() + text.len(), text.len())?;
            Rc::make_mut(a).push_str(&text);
        }
        (Value::Arr(a), Value::Arr(b)) => {
            meter.grow_coll(a.len() + b.len(), b.len())?;
            if a.is_empty() {
                *a = b;
            } else {
                a.make_mut().extend(b.into_vec());
            }
        }
        (Value::Arr(a), b) => {
            meter.grow_coll(a.len() + 1, 1)?;
            a.make_mut().push(b);
        }
        (Value::Map(a), Value::Map(b)) => {
            meter.grow_coll(a.len() + b.len(), b.len())?;
            if a.is_empty() {
                *a = b;
            } else {
                a.make_mut().extend(b.into_map());
            }
        }
        (Value::Bool(a), Value::Bool(b)) => *a = *a && b,
        (left @ (Value::Int(_) | Value::Float(_) | Value::Bool(_)), Value::Str(b)) => {
            let mut text = left.to_string();
            meter.str_len(text.len() + b.len())?;
            text.push_str(&b);
            *left = Value::str(text);
        }
        (left, Value::Arr(b)) => {
            meter.coll_len(b.len() + 1)?;
            let mut items = Vec::with_capacity(b.len() + 1);
            items.push(std::mem::take(left));
            items.extend(b.into_vec());
            *left = Value::arr(items);
        }
        (left, right) => {
            return Err(Error::type_error(format!(
                "can't add {} to {}",
                right.type_name(),
                left.type_name()
            )));
        }
    }
    Ok(())
}

/// `a + b`. See [`add_assign`].
pub fn add(mut a: Value, b: Value, meter: &mut Meter) -> Result<Value> {
    add_assign(&mut a, b, meter)?;
    Ok(a)
}

fn arith(
    a: &Value,
    b: &Value,
    symbol: &str,
    int: fn(i64, i64) -> Option<i64>,
    float: fn(f64, f64) -> f64,
) -> Result<Value> {
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        if let Some(r) = int(*x, *y) {
            return Ok(Value::Int(r));
        }
    }
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => Ok(Value::num(float(x, y))),
        _ => Err(Error::type_error(format!(
            "can't compute {} {symbol} {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

/// `a - b`.
pub fn sub(a: &Value, b: &Value) -> Result<Value> {
    arith(a, b, "-", i64::checked_sub, |x, y| x - y)
}

/// `a * b`. Also repeats strings and arrays: `"ab" * 3`, `[x] * 2`.
pub fn mul(a: &Value, b: &Value, meter: &mut Meter) -> Result<Value> {
    match (a, b) {
        (Value::Str(s), n @ (Value::Int(_) | Value::Float(_)))
        | (n @ (Value::Int(_) | Value::Float(_)), Value::Str(s)) => {
            let n = repeat_count(n)?;
            meter.str_len(s.len().saturating_mul(n))?;
            Ok(Value::str(s.repeat(n)))
        }
        (Value::Arr(items), n @ (Value::Int(_) | Value::Float(_)))
        | (n @ (Value::Int(_) | Value::Float(_)), Value::Arr(items)) => {
            let n = repeat_count(n)?;
            meter.coll_len(items.len().saturating_mul(n))?;
            Ok(Value::arr(
                items
                    .iter()
                    .cloned()
                    .cycle()
                    .take(items.len() * n)
                    .collect(),
            ))
        }
        _ => arith(a, b, "*", i64::checked_mul, |x, y| x * y),
    }
}

fn repeat_count(n: &Value) -> Result<usize> {
    match n {
        Value::Int(i) if *i >= 0 => Ok(*i as usize),
        _ => Err(Error::value_error(format!(
            "can't repeat {} times",
            n.repr()
        ))),
    }
}

/// `a / b`. Whole results stay whole; division by zero gives `inf` or `NaN`.
pub fn div(a: &Value, b: &Value) -> Result<Value> {
    arith(
        a,
        b,
        "/",
        |x, y| {
            if y != 0 && x % y == 0 {
                x.checked_div(y)
            } else {
                None
            }
        },
        |x, y| x / y,
    )
}

/// `a % b`, with the sign of the divisor (so `-1 % 3 == 2`).
pub fn rem(a: &Value, b: &Value) -> Result<Value> {
    arith(
        a,
        b,
        "%",
        |x, y| {
            if y == 0 {
                None
            } else {
                Some(x.checked_rem_euclid(y).unwrap_or(0))
            }
        },
        |x, y| x - y * (x / y).floor(),
    )
}

/// `-a`.
pub fn neg(a: &Value) -> Result<Value> {
    match a {
        Value::Int(i) => Ok(i
            .checked_neg()
            .map(Value::Int)
            .unwrap_or_else(|| Value::num(-(*i as f64)))),
        Value::Float(f) => Ok(Value::Float(-f)),
        other => Err(Error::type_error(format!(
            "can't negate {}",
            other.type_name()
        ))),
    }
}

/// Orders two numbers or two strings. Anything else is a type error.
pub fn compare(a: &Value, b: &Value) -> Result<Option<Ordering>> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(Some(x.cmp(y))),
        (Value::Str(x), Value::Str(y)) => Ok(Some(x.cmp(y))),
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => Ok(x.partial_cmp(&y)),
            _ => Err(Error::type_error(format!(
                "can't compare {} with {}",
                a.type_name(),
                b.type_name()
            ))),
        },
    }
}

/// `a..b`: the whole numbers from `a` up to (excluding) `b`, as an array.
pub fn range(a: &Value, b: &Value, meter: &mut Meter) -> Result<Value> {
    match (a, b) {
        (Value::Int(lo), Value::Int(hi)) => {
            let len = hi.saturating_sub(*lo).max(0) as usize;
            meter.coll_len(len)?;
            Ok(Value::arr((*lo..*hi).map(Value::Int).collect()))
        }
        _ => Err(Error::type_error(format!(
            "a range needs two whole numbers, not {} and {}",
            a.describe(),
            b.describe()
        ))),
    }
}

/// The text of a map key: strings as is, anything else by display.
pub fn key_text(key: &Value) -> Rc<str> {
    match key {
        Value::Str(s) => Rc::from(s.as_str()),
        other => Rc::from(other.to_string().as_str()),
    }
}

fn int_key(key: &Value) -> Option<i64> {
    match key {
        Value::Int(i) => Some(*i),
        Value::Str(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Resolves a possibly negative index into `0..len` (or `None` when out of range).
fn resolve_index(i: i64, len: usize) -> Option<usize> {
    let i = if i < 0 { i + len as i64 } else { i };
    (0..len as i64).contains(&i).then_some(i as usize)
}

/// `value.key` / `value[key]`. Missing keys and out-of-range indices give
/// `null`, as does any key of `null` (so `$a.b.c ?? default` works).
pub fn get_key(value: &Value, key: &Value) -> Result<Value> {
    match value {
        Value::Map(map) => Ok(map.get(&*key_text(key)).cloned().unwrap_or_default()),
        Value::Arr(items) => match int_key(key) {
            Some(i) => Ok(resolve_index(i, items.len())
                .map(|i| items[i].clone())
                .unwrap_or_default()),
            None => Err(Error::type_error(format!(
                "arrays are indexed by whole numbers, not {}",
                key.describe()
            ))),
        },
        Value::Str(s) => match int_key(key) {
            Some(i) => {
                let len = s.chars().count();
                Ok(resolve_index(i, len)
                    .and_then(|i| s.chars().nth(i))
                    .map(|c| Value::str(c.to_string()))
                    .unwrap_or_default())
            }
            None => Err(Error::type_error(format!(
                "strings are indexed by whole numbers, not {}",
                key.describe()
            ))),
        },
        Value::Null => Ok(Value::Null),
        other => Err(Error::type_error(format!(
            "can't look up {} in {}",
            key.quoted_repr(),
            super::with_article(other.type_name())
        ))),
    }
}

/// Reads `root` followed by a path of keys.
pub fn get_path(root: &Value, keys: &[Value]) -> Result<Value> {
    let mut current = root.clone();
    for key in keys {
        current = get_key(&current, key)?;
    }
    Ok(current)
}

/// How an assignment combines the new value with the old one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UpdateOp {
    /// `=`
    Set,
    /// `+=`
    Add,
    /// `-=`
    Sub,
    /// `*=`
    Mul,
    /// `/=`
    Div,
}

/// Applies `op` to the place reached by following `keys` from `root`, in place.
///
/// Missing map keys are created; `null` becomes a map when given a field; an
/// array index equal to the length appends.
pub fn update_path(
    root: &mut Value,
    keys: &[Value],
    op: UpdateOp,
    value: Value,
    meter: &mut Meter,
) -> Result<()> {
    let mut place = root;
    for key in keys {
        place = child_mut(place, key, meter)?;
    }
    match op {
        UpdateOp::Set => *place = value,
        UpdateOp::Add => add_assign(place, value, meter)?,
        UpdateOp::Sub => *place = sub(place, &value)?,
        UpdateOp::Mul => *place = mul(place, &value, meter)?,
        UpdateOp::Div => *place = div(place, &value)?,
    }
    Ok(())
}

fn child_mut<'a>(place: &'a mut Value, key: &Value, meter: &mut Meter) -> Result<&'a mut Value> {
    if place.is_null() {
        *place = Value::map(Map::default());
    }
    match place {
        Value::Map(obj) => {
            let key = key_text(key);
            let map = obj.make_mut();
            if !map.contains_key(&key) {
                meter.grow_coll(map.len() + 1, 1)?;
            }
            Ok(map.entry(key).or_default())
        }
        Value::Arr(arr) => {
            let Some(i) = int_key(key) else {
                return Err(Error::type_error(format!(
                    "arrays are indexed by whole numbers, not {}",
                    key.describe()
                )));
            };
            let items = arr.make_mut();
            let len = items.len();
            if i == len as i64 {
                meter.grow_coll(len + 1, 1)?;
                items.push(Value::Null);
                return Ok(&mut items[len]);
            }
            match resolve_index(i, len) {
                Some(i) => Ok(&mut items[i]),
                None => Err(Error::value_error(format!(
                    "index {i} is out of range for an array of length {len}"
                ))),
            }
        }
        other => Err(Error::type_error(format!(
            "can't assign into {}",
            super::with_article(other.type_name())
        ))),
    }
}
