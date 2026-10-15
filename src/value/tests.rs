use proptest::prelude::*;

use super::*;
use crate::error::{ErrorKind, Limit};
use crate::limits::{Limits, Meter};

fn s(text: &str) -> Value {
    Value::str(text)
}

fn arr(items: &[Value]) -> Value {
    Value::arr(items.to_vec())
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::map(
        entries
            .iter()
            .map(|(k, v)| ((*k).into(), v.clone()))
            .collect(),
    )
}

fn plus(a: Value, b: Value) -> Result<Value, ErrorKind> {
    add(a, b, &mut Meter::unlimited()).map_err(|e| e.kind())
}

#[test]
fn value_is_sixteen_bytes() {
    assert_eq!(std::mem::size_of::<Value>(), 16);
}

#[test]
fn fold_matrix() {
    let i = Value::Int;
    let cases: Vec<(Value, Value, Value)> = vec![
        (Value::Null, i(1), i(1)),
        (i(1), Value::Null, i(1)),
        (Value::Null, Value::Null, Value::Null),
        (i(1), i(2), i(3)),
        (i(1), Value::num(0.5), Value::num(1.5)),
        (Value::num(0.5), Value::num(0.5), i(1)),
        (s("ab"), s("cd"), s("abcd")),
        (s("hi"), i(5), s("hi5")),
        (i(5), s("hi"), s("5hi")),
        (s("is "), Value::Bool(true), s("is true")),
        (Value::Bool(false), s("!"), s("false!")),
        (arr(&[s("a")]), arr(&[s("b")]), arr(&[s("a"), s("b")])),
        (arr(&[s("a")]), i(1), arr(&[s("a"), i(1)])),
        (i(1), arr(&[s("a")]), arr(&[i(1), s("a")])),
        (s("x"), arr(&[]), arr(&[s("x")])),
        (arr(&[]), map(&[("k", i(1))]), arr(&[map(&[("k", i(1))])])),
        (
            map(&[("a", i(1)), ("b", i(2))]),
            map(&[("b", i(3)), ("c", i(4))]),
            map(&[("a", i(1)), ("b", i(3)), ("c", i(4))]),
        ),
        (Value::Bool(true), Value::Bool(true), Value::Bool(true)),
        (Value::Bool(true), Value::Bool(false), Value::Bool(false)),
        (Value::Bool(false), Value::Bool(true), Value::Bool(false)),
    ];
    for (a, b, expected) in cases {
        let label = format!("{a:?} + {b:?}");
        assert_eq!(plus(a, b).unwrap(), expected, "{label}");
    }
}

#[test]
fn fold_type_errors() {
    for (a, b) in [
        (map(&[]), s("x")),
        (s("x"), map(&[])),
        (Value::Int(1), map(&[])),
        (Value::Bool(true), Value::Int(1)),
        (Value::Int(1), Value::Bool(true)),
    ] {
        assert_eq!(plus(a, b), Err(ErrorKind::Type));
    }
}

#[test]
fn fold_error_message_names_types() {
    let err = add(Value::Int(1), map(&[]), &mut Meter::unlimited()).unwrap_err();
    assert_eq!(err.message(), "can't add map to num");
}

#[test]
fn integer_overflow_promotes_to_float() {
    let big = plus(Value::Int(i64::MAX), Value::Int(1)).unwrap();
    assert!(matches!(big, Value::Float(_)));
    assert_eq!(
        mul(
            &Value::Int(i64::MAX),
            &Value::Int(2),
            &mut Meter::unlimited()
        )
        .unwrap()
        .type_name(),
        "num"
    );
}

#[test]
fn num_normalises_integral_floats() {
    assert!(matches!(Value::num(3.0), Value::Int(3)));
    assert!(matches!(Value::num(-0.0), Value::Int(0)));
    assert!(matches!(Value::num(2.5), Value::Float(_)));
    assert!(matches!(Value::num(1e300), Value::Float(_)));
    assert!(matches!(Value::num(f64::NAN), Value::Float(_)));
}

#[test]
fn arithmetic() {
    let i = Value::Int;
    let m = &mut Meter::unlimited();
    assert_eq!(sub(&i(5), &i(7)).unwrap(), i(-2));
    assert_eq!(div(&i(6), &i(3)).unwrap(), i(2));
    assert_eq!(div(&i(7), &i(2)).unwrap(), Value::num(3.5));
    assert_eq!(div(&i(1), &i(0)).unwrap().to_string(), "inf");
    assert_eq!(div(&i(0), &i(0)).unwrap().to_string(), "NaN");
    assert_eq!(rem(&i(-1), &i(3)).unwrap(), i(2));
    assert_eq!(rem(&i(7), &i(3)).unwrap(), i(1));
    assert_eq!(rem(&Value::num(5.5), &i(2)).unwrap(), Value::num(1.5));
    assert_eq!(neg(&i(3)).unwrap(), i(-3));
    assert_eq!(mul(&s("ab"), &i(3), m).unwrap(), s("ababab"));
    assert_eq!(mul(&i(2), &arr(&[i(1)]), m).unwrap(), arr(&[i(1), i(1)]));
    assert_eq!(
        mul(&s("ab"), &i(-1), m).unwrap_err().kind(),
        ErrorKind::Value
    );
    assert_eq!(sub(&s("a"), &i(1)).unwrap_err().kind(), ErrorKind::Type);
}

#[test]
fn repeat_respects_size_limit() {
    let limits = Limits {
        max_str_bytes: 100,
        ..Limits::default()
    };
    let mut meter = Meter::new(&limits, Default::default());
    let err = mul(&s("a"), &Value::Int(1_000_000_000_000), &mut meter).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Limit(Limit::StrSize));
}

#[test]
fn comparisons() {
    use std::cmp::Ordering::*;
    assert_eq!(
        compare(&Value::Int(1), &Value::num(1.5)).unwrap(),
        Some(Less)
    );
    assert_eq!(compare(&s("b"), &s("a")).unwrap(), Some(Greater));
    assert_eq!(
        compare(&Value::num(f64::NAN), &Value::Int(1)).unwrap(),
        None
    );
    assert!(compare(&s("1"), &Value::Int(1)).is_err());
}

#[test]
fn equality_is_structural_and_typed() {
    assert_eq!(Value::Int(1), Value::num(1.0));
    assert_ne!(s("1"), Value::Int(1));
    assert_eq!(arr(&[s("a"), Value::Int(1)]), arr(&[s("a"), Value::Int(1)]));
    assert_eq!(
        map(&[("a", Value::Int(1)), ("b", Value::Int(2))]),
        map(&[("b", Value::Int(2)), ("a", Value::Int(1))])
    );
    assert_ne!(Value::num(f64::NAN), Value::num(f64::NAN));
}

#[test]
fn truthiness() {
    for falsy in [
        Value::Null,
        Value::Bool(false),
        Value::Int(0),
        Value::num(0.0),
        s(""),
        arr(&[]),
        map(&[]),
    ] {
        assert!(!falsy.truthy(), "{falsy:?} should be falsy");
    }
    for truthy in [
        Value::Bool(true),
        Value::Int(-1),
        s("0"),
        arr(&[Value::Null]),
    ] {
        assert!(truthy.truthy(), "{truthy:?} should be truthy");
    }
}

#[test]
fn display_and_repr() {
    assert_eq!(s("hi").to_string(), "hi");
    assert_eq!(Value::Null.to_string(), "");
    assert_eq!(Value::Null.repr(), "null");
    assert_eq!(Value::num(0.1 + 0.2).to_string(), "0.30000000000000004");
    assert_eq!(Value::num(1e21).to_string(), "1e21");
    assert_eq!(Value::num(-2.5).to_string(), "-2.5");
    assert_eq!(
        arr(&[s("a"), Value::Int(1), Value::Null, s("q\"\n")]).to_string(),
        r#"["a", 1, null, "q\"\n"]"#
    );
    assert_eq!(
        map(&[("k", arr(&[])), ("x", map(&[]))]).to_string(),
        r#"{"k": [], "x": {}}"#
    );
}

#[test]
fn display_of_deep_nesting_is_bounded() {
    let mut v = Value::Null;
    for _ in 0..10_000 {
        v = arr(&[v]);
    }
    assert!(v.to_string().contains('…'));
}

#[test]
fn deep_values_drop_without_overflow() {
    let mut v = Value::Null;
    for _ in 0..1_000_000 {
        v = Value::arr(vec![v]);
    }
    let mut m = Value::Null;
    for _ in 0..1_000_000 {
        let mut map = Map::default();
        map.insert("x".into(), m);
        m = Value::map(map);
    }
    drop(v);
    drop(m);
}

#[test]
fn coercions() {
    let num = Type::NUM;
    assert_eq!(coerce(s("21"), num).unwrap(), Value::Int(21));
    assert_eq!(coerce(s(" 2.5 "), num).unwrap(), Value::num(2.5));
    assert_eq!(coerce(s("inf"), num).unwrap().to_string(), "inf");
    assert!(coerce(s("abc"), num).is_err());
    assert!(coerce(s(""), num).is_err());
    assert_eq!(coerce(Value::Int(5), Type::STR).unwrap(), s("5"));
    assert_eq!(coerce(Value::Bool(true), Type::STR).unwrap(), s("true"));
    assert_eq!(
        coerce(s(" a  b "), Type::ARR).unwrap(),
        arr(&[s("a"), s("b")])
    );
    assert_eq!(coerce(s(""), Type::BOOL).unwrap(), Value::Bool(false));
    assert!(coerce(map(&[]), Type::STR).is_err());
    assert!(coerce(Value::Null, Type::STR).is_err());
    // unions try num before str
    assert_eq!(
        coerce(s("7"), Type::NUM.union(Type::ARR)).unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        coerce(s("x y"), Type::NUM.union(Type::ARR)).unwrap(),
        arr(&[s("x"), s("y")])
    );
}

#[test]
fn can_coerce_agrees_with_coerce() {
    let values = [
        Value::Null,
        Value::Bool(true),
        Value::Int(3),
        Value::num(2.5),
        s("12"),
        s("word"),
        arr(&[]),
        map(&[]),
    ];
    let types = [
        Type::NULL,
        Type::BOOL,
        Type::NUM,
        Type::STR,
        Type::ARR,
        Type::MAP,
        Type::ANY,
        Type::NUM.union(Type::NULL),
    ];
    for v in &values {
        for t in types {
            assert_eq!(
                can_coerce(v, t),
                coerce(v.clone(), t).is_ok(),
                "{v:?} to {t}"
            );
        }
    }
}

#[test]
fn type_names_display() {
    assert_eq!(Type::ANY.to_string(), "any");
    assert_eq!(Type::NUM.union(Type::NULL).to_string(), "num?");
    assert_eq!(Type::STR.union(Type::NUM).to_string(), "num|str");
    assert_eq!(Type::from_name("map"), Some(Type::MAP));
    assert_eq!(Type::from_name("nope"), None);
}

#[test]
fn keys_and_indexing() {
    let list = arr(&[s("a"), s("b"), s("c")]);
    assert_eq!(get_key(&list, &Value::Int(0)).unwrap(), s("a"));
    assert_eq!(get_key(&list, &Value::Int(-1)).unwrap(), s("c"));
    assert_eq!(get_key(&list, &s("1")).unwrap(), s("b"));
    assert_eq!(get_key(&list, &Value::Int(9)).unwrap(), Value::Null);
    assert!(get_key(&list, &s("x")).is_err());
    let m = map(&[("name", s("Bob"))]);
    assert_eq!(get_key(&m, &s("name")).unwrap(), s("Bob"));
    assert_eq!(get_key(&m, &s("age")).unwrap(), Value::Null);
    assert_eq!(get_key(&Value::Null, &s("x")).unwrap(), Value::Null);
    assert_eq!(get_key(&s("cão"), &Value::Int(1)).unwrap(), s("ã"));
    assert!(get_key(&Value::Int(1), &s("x")).is_err());
}

#[test]
fn update_paths() {
    let m = &mut Meter::unlimited();
    let mut root = Value::Null;
    update_path(
        &mut root,
        &[s("a"), s("b")],
        UpdateOp::Set,
        Value::Int(1),
        m,
    )
    .unwrap();
    assert_eq!(root, map(&[("a", map(&[("b", Value::Int(1))]))]));
    update_path(
        &mut root,
        &[s("a"), s("b")],
        UpdateOp::Add,
        Value::Int(2),
        m,
    )
    .unwrap();
    assert_eq!(get_path(&root, &[s("a"), s("b")]).unwrap(), Value::Int(3));

    let mut list = arr(&[Value::Int(1)]);
    update_path(&mut list, &[Value::Int(1)], UpdateOp::Set, Value::Int(2), m).unwrap();
    update_path(
        &mut list,
        &[Value::Int(-1)],
        UpdateOp::Mul,
        Value::Int(10),
        m,
    )
    .unwrap();
    assert_eq!(list, arr(&[Value::Int(1), Value::Int(20)]));
    assert!(update_path(&mut list, &[Value::Int(5)], UpdateOp::Set, Value::Null, m).is_err());
    assert!(update_path(&mut s("x"), &[Value::Int(0)], UpdateOp::Set, Value::Null, m).is_err());
}

#[test]
fn copy_on_write_keeps_values_independent() {
    let m = &mut Meter::unlimited();
    let original = arr(&[Value::Int(1)]);
    let mut copy = original.clone();
    add_assign(&mut copy, Value::Int(2), m).unwrap();
    assert_eq!(original, arr(&[Value::Int(1)]));
    assert_eq!(copy, arr(&[Value::Int(1), Value::Int(2)]));
}

#[test]
fn range_values() {
    let m = &mut Meter::unlimited();
    assert_eq!(
        range(&Value::Int(1), &Value::Int(4), m).unwrap(),
        arr(&[Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(range(&Value::Int(4), &Value::Int(1), m).unwrap(), arr(&[]));
    assert!(range(&Value::num(0.5), &Value::Int(1), m).is_err());
}

#[test]
fn json_round_trip_and_errors() {
    let v = parse_json(r#"{"a": [1, 2.5, "x", null, true], "b": {}}"#).unwrap();
    assert_eq!(
        v.to_string(),
        r#"{"a": [1, 2.5, "x", null, true], "b": {}}"#
    );
    let json = to_json(&v).unwrap();
    assert_eq!(from_json(json), v);
    assert!(parse_json("{").is_err());
    assert!(to_json(&Value::num(f64::NAN)).is_err());
    let deep = "[".repeat(10_000);
    assert!(parse_json(&deep).is_err());
}

fn scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        (-1000i64..1000).prop_map(Value::Int),
        "[a-z ]{0,6}".prop_map(Value::str),
    ]
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        (-1e9f64..1e9).prop_map(Value::num),
        ".{0,8}".prop_map(Value::str),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::arr),
            prop::collection::vec(("[a-z]{0,4}", inner), 0..6).prop_map(|entries| {
                Value::map(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
            }),
        ]
    })
}

proptest! {
    #[test]
    fn null_is_identity(v in json_value()) {
        prop_assert_eq!(plus(Value::Null, v.clone()).unwrap(), v.clone());
        prop_assert_eq!(plus(v.clone(), Value::Null).unwrap(), v);
    }

    #[test]
    fn fold_is_associative_within_a_type(
        (a, b, c) in prop_oneof![
            (any::<i32>(), any::<i32>(), any::<i32>())
                .prop_map(|(a, b, c)| (Value::Int(a.into()), Value::Int(b.into()), Value::Int(c.into()))),
            ("[a-z]{0,5}", "[a-z]{0,5}", "[a-z]{0,5}")
                .prop_map(|(a, b, c)| (Value::str(a), Value::str(b), Value::str(c))),
            (prop::collection::vec(scalar(), 0..4), prop::collection::vec(scalar(), 0..4), prop::collection::vec(scalar(), 0..4))
                .prop_map(|(a, b, c)| (Value::arr(a), Value::arr(b), Value::arr(c))),
            (any::<bool>(), any::<bool>(), any::<bool>())
                .prop_map(|(a, b, c)| (Value::Bool(a), Value::Bool(b), Value::Bool(c))),
            (prop::collection::vec(("[a-c]", scalar()), 0..4), prop::collection::vec(("[a-c]", scalar()), 0..4), prop::collection::vec(("[a-c]", scalar()), 0..4))
                .prop_map(|(a, b, c)| {
                    let mk = |e: Vec<(String, Value)>| Value::map(e.into_iter().map(|(k, v)| (k.into(), v)).collect());
                    (mk(a), mk(b), mk(c))
                }),
        ]
    ) {
        let left = plus(plus(a.clone(), b.clone()).unwrap(), c.clone()).unwrap();
        let right = plus(a, plus(b, c).unwrap()).unwrap();
        prop_assert_eq!(left, right);
    }

    #[test]
    fn json_round_trips(v in json_value()) {
        let back = from_json(to_json(&v).unwrap());
        prop_assert_eq!(back, v);
    }

    #[test]
    fn repr_parses_as_the_same_json(v in json_value()) {
        prop_assert_eq!(parse_json(&v.repr()).unwrap(), v);
    }

    #[test]
    fn parse_num_never_panics(text in ".{0,12}") {
        let _ = parse_num(&text);
    }
}
