//! Sequences. These work on arrays and, dog3-style, on text split into words:
//! give them a string and you get a string back.

use super::{Library, checked_str, int, text};
use crate::error::{Error, Result};
use crate::value::{Value, compare};
use crate::vm::Vm;

/// A sequence argument: an array, or words of a string with the separator to
/// join them back with.
enum Seq {
    Items(Vec<Value>),
    Words(Vec<String>, String),
}

fn seq(value: &Value, sep: Option<&Value>) -> Result<Seq> {
    match (value, sep) {
        (Value::Arr(items), None) => Ok(Seq::Items(items.to_vec())),
        (Value::Null, None) => Ok(Seq::Items(Vec::new())),
        (Value::Str(s), None) => Ok(Seq::Words(
            s.split_whitespace().map(str::to_string).collect(),
            " ".into(),
        )),
        (Value::Str(s), Some(sep)) => {
            let sep = sep.to_string();
            let words = if s.is_empty() {
                Vec::new()
            } else if sep.is_empty() {
                s.chars().map(String::from).collect()
            } else {
                s.split(sep.as_str()).map(str::to_string).collect()
            };
            Ok(Seq::Words(words, sep))
        }
        (other, None) => Err(Error::type_error(format!(
            "expected an array or text, got {}",
            other.describe()
        ))),
        (other, Some(_)) => Err(Error::type_error(format!(
            "a separator only works with text, not {}",
            other.describe()
        ))),
    }
}

impl Seq {
    fn len(&self) -> usize {
        match self {
            Seq::Items(items) => items.len(),
            Seq::Words(words, _) => words.len(),
        }
    }

    fn slice(self, vm: &mut Vm, range: std::ops::Range<usize>) -> Result<Value> {
        match self {
            Seq::Items(items) => Ok(Value::arr(items[range].to_vec())),
            Seq::Words(words, sep) => checked_str(vm, words[range].join(&sep)),
        }
    }
}

fn first(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let n = count(&args[1])?;
    let seq = seq(&args[0], args.get(2))?;
    if n > seq.len() {
        return Ok(Value::Null);
    }
    seq.slice(vm, 0..n)
}

fn last(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let n = count(&args[1])?;
    let seq = seq(&args[0], args.get(2))?;
    let len = seq.len();
    if n > len {
        return Ok(Value::Null);
    }
    seq.slice(vm, len - n..len)
}

fn count(value: &Value) -> Result<usize> {
    let n = int(value, "a count")?;
    usize::try_from(n).map_err(|_| Error::value_error(format!("a count can't be negative ({n})")))
}

fn append(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    match (seq(&args[0], args.get(2))?, seq(&args[1], args.get(2))?) {
        (Seq::Items(mut a), Seq::Items(b)) => {
            vm.meter().coll_len(a.len() + b.len())?;
            a.extend(b);
            Ok(Value::arr(a))
        }
        (Seq::Words(mut a, sep), Seq::Words(b, _)) => {
            a.extend(b);
            checked_str(vm, a.join(&sep))
        }
        _ => Err(Error::type_error("`append` needs two arrays or two texts")),
    }
}

fn range(vm: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let (lo, hi, step) = match args.len() {
        1 => (0, int(&args[0], "a range bound")?, 1),
        _ => (
            int(&args[0], "a range bound")?,
            int(&args[1], "a range bound")?,
            match args.get(2) {
                Some(step) => int(step, "a range step")?,
                None => 1,
            },
        ),
    };
    if step == 0 {
        return Err(Error::value_error("a range step can't be 0"));
    }
    let span = if step > 0 {
        hi.saturating_sub(lo)
    } else {
        lo.saturating_sub(hi)
    };
    let len = if span <= 0 {
        0
    } else {
        (span - 1) / step.abs() + 1
    } as usize;
    vm.meter().coll_len(len)?;
    let numbers = (0..len as i64).map(|i| lo + i * step);
    match args.get(3) {
        Some(sep) => {
            let sep = sep.to_string();
            let words: Vec<String> = numbers.map(|n| n.to_string()).collect();
            checked_str(vm, words.join(&sep))
        }
        None => Ok(Value::arr(numbers.map(Value::Int).collect())),
    }
}

fn len(_: &mut Vm, args: &mut [Value]) -> Result<Value> {
    let n = match (&args[0], args.get(1)) {
        (Value::Map(map), None) => map.len(),
        // Counted in place: `seq` would copy the whole array.
        (Value::Arr(items), None) => items.len(),
        (value, sep) => seq(value, sep)?.len(),
    };
    Ok(Value::Int(n as i64))
}

pub(super) fn install(lib: &mut Library) {
    lib.def("len", "items", len);
    lib.def("len", "items, sep", len);
    lib.def("first", "items, n: num", first);
    lib.def("first", "items, n: num, sep", first);
    lib.def("last", "items, n: num", last);
    lib.def("last", "items, n: num, sep", last);
    lib.def("append", "left, right", append);
    lib.def("append", "left, right, sep", append);
    lib.def("range", "max: num", range);
    lib.def("range", "min: num, max: num", range);
    lib.def("range", "min: num, max: num, step: num", range);
    lib.def("range", "min: num, max: num, step: num, sep", range);
    lib.def("keys", "map: map", |_, args| {
        let Value::Map(map) = &args[0] else {
            return Ok(Value::Null);
        };
        Ok(Value::arr(map.keys().map(|k| Value::str(&**k)).collect()))
    });
    lib.def("values", "map: map", |_, args| {
        let Value::Map(map) = &args[0] else {
            return Ok(Value::Null);
        };
        Ok(Value::arr(map.values().cloned().collect()))
    });
    lib.def("contains", "haystack, needle", |_, args| {
        let found = match (&args[0], &args[1]) {
            (Value::Arr(items), needle) => items.contains(needle),
            (Value::Map(map), key) => map.contains_key(&*crate::value::key_text(key)),
            (Value::Str(s), needle) => s.contains(&needle.to_string()),
            (Value::Null, _) => false,
            (other, _) => {
                return Err(Error::type_error(format!(
                    "can't search inside {}",
                    other.describe()
                )));
            }
        };
        Ok(Value::Bool(found))
    });
    lib.def("join", "items: arr", |vm, args| {
        checked_str(vm, super::join_display(super::items(&args[0]), " "))
    });
    lib.def("join", "items: arr, sep: str", |vm, args| {
        let sep = text(&args[1]).to_string();
        checked_str(vm, super::join_display(super::items(&args[0]), &sep))
    });
    lib.def("split", "text: str", |vm, args| {
        let words: Vec<Value> = text(&args[0]).split_whitespace().map(Value::from).collect();
        vm.meter().coll_len(words.len())?;
        Ok(Value::arr(words))
    });
    lib.def("split", "text: str, sep: str", |vm, args| {
        let Seq::Words(words, _) = seq(&args[0], Some(&args[1]))? else {
            unreachable!()
        };
        vm.meter().coll_len(words.len())?;
        Ok(Value::arr(words.into_iter().map(Value::from).collect()))
    });
    lib.def("chars", "text: str", |vm, args| {
        let chars: Vec<Value> = text(&args[0])
            .chars()
            .map(|c| Value::str(c.to_string()))
            .collect();
        vm.meter().coll_len(chars.len())?;
        Ok(Value::arr(chars))
    });
    lib.def("reverse", "items", |vm, args| match &args[0] {
        Value::Arr(items) => Ok(Value::arr(items.iter().rev().cloned().collect())),
        Value::Str(s) => checked_str(vm, s.chars().rev().collect()),
        other => Err(Error::type_error(format!(
            "can't reverse {}",
            other.describe()
        ))),
    });
    lib.def("sum", "items: arr", |vm, args| {
        let mut total = Value::Null;
        for item in super::take_items(&mut args[0]) {
            crate::value::add_assign(&mut total, item, vm.meter())?;
        }
        Ok(total)
    });
    lib.def("sort", "items: arr", |vm, args| {
        let mut items = super::take_items(&mut args[0]);
        vm.meter().tick(items.len() as u64)?;
        sort_values(&mut items, |v| v.clone())?;
        Ok(Value::arr(items))
    });
    lib.def("index_of", "items, needle", |_, args| {
        let found = match (&args[0], &args[1]) {
            (Value::Arr(items), needle) => items.iter().position(|x| x == needle),
            (Value::Str(s), needle) => {
                let needle = needle.to_string();
                s.find(&needle).map(|byte| s[..byte].chars().count())
            }
            (other, _) => {
                return Err(Error::type_error(format!(
                    "can't search inside {}",
                    other.describe()
                )));
            }
        };
        Ok(found.map_or(Value::Null, |i| Value::Int(i as i64)))
    });
}

/// Sorts by a key, failing (instead of panicking) on incomparable keys.
pub(crate) fn sort_values(items: &mut [Value], key: impl Fn(&Value) -> Value) -> Result<()> {
    let mut error = None;
    items.sort_by(|a, b| match compare(&key(a), &key(b)) {
        Ok(Some(ordering)) => ordering,
        Ok(None) => std::cmp::Ordering::Equal,
        Err(e) => {
            error.get_or_insert(e);
            std::cmp::Ordering::Equal
        }
    });
    error.map_or(Ok(()), Err)
}
