//! What `for` loops iterate over.

use crate::error::{Error, Result};
use crate::limits::Meter;
use crate::value::{Array, Object, Value, with_article};

/// The state of one running `for` loop.
pub enum Iter {
    /// Array elements (index, element).
    Items { items: Array, next: usize },
    /// Map entries (key, value).
    Entries { map: Object, next: usize },
}

impl Iter {
    /// Starts iterating `value`, optionally splitting it as text first.
    ///
    /// * arrays: their elements
    /// * maps: their keys (or key and value)
    /// * strings: whitespace-separated words, or pieces between `split` separators
    ///   (an empty separator splits into characters)
    /// * `null`: nothing
    pub fn new(value: Value, split: Option<Value>, meter: &mut Meter) -> Result<Iter> {
        let items = match (value, split) {
            (Value::Arr(items), None) => items,
            (Value::Map(map), None) => return Ok(Iter::Entries { map, next: 0 }),
            (Value::Null, _) => Array::default(),
            (Value::Str(text), sep) => {
                let pieces: Vec<Value> = match sep.as_ref().map(|s| s.to_string()) {
                    None => text.split_whitespace().map(Value::from).collect(),
                    Some(sep) if sep.is_empty() => {
                        text.chars().map(|c| Value::str(c.to_string())).collect()
                    }
                    Some(sep) => text.split(sep.as_str()).map(Value::from).collect(),
                };
                meter.coll_len(pieces.len())?;
                Array::new(pieces)
            }
            (other, None) => {
                return Err(Error::type_error(format!(
                    "can't loop over {}",
                    with_article(other.type_name())
                )));
            }
            (other, Some(_)) => {
                return Err(Error::type_error(format!(
                    "`split` needs a string, not {}",
                    with_article(other.type_name())
                )));
            }
        };
        Ok(Iter::Items { items, next: 0 })
    }

    /// The next element: `(element, None)`, or `(key, Some(element))` for pairs.
    pub fn next(&mut self, pairs: bool) -> Option<(Value, Option<Value>)> {
        match self {
            Iter::Items { items, next } => {
                let item = items.get(*next)?.clone();
                let index = Value::Int(*next as i64);
                *next += 1;
                Some(if pairs {
                    (index, Some(item))
                } else {
                    (item, None)
                })
            }
            Iter::Entries { map, next } => {
                let (key, value) = map.get_index(*next)?;
                *next += 1;
                let key = Value::str(&**key);
                Some(if pairs {
                    (key, Some(value.clone()))
                } else {
                    (key, None)
                })
            }
        }
    }
}
