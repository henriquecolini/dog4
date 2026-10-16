//! Fuzz-style tests: random programs must never panic, hang or escape their
//! limits. Errors are fine; crashes are not. (The `fuzz/` directory has the
//! same harness for `cargo fuzz`, which needs a nightly toolchain.)

use std::time::Duration;

use dog4::{Builder, Limits, MemoryProvider, OutputBuffer};
use proptest::prelude::*;

fn tight_limits() -> Limits {
    Limits {
        fuel: Some(20_000),
        timeout: Some(Duration::from_secs(2)),
        max_call_depth: 64,
        max_native_depth: 8,
        max_str_bytes: 10_000,
        max_collection_len: 10_000,
        max_output_bytes: Some(10_000),
        max_nesting: 32,
    }
}

fn run(source: &str) {
    let modules = MemoryProvider::new()
        .with("m", "pub x = 1; pub fn f (a) { put $a }")
        .with("loop", "use mod loop");
    let mut rt = Builder::new()
        .limits(tight_limits())
        .output(OutputBuffer::default())
        .provider(modules)
        .seed(1)
        .build();
    if let Err(e) = rt.run("fuzz", source) {
        // Rendering the error must not panic either.
        let _ = e.to_string();
    }
}

const TOKENS: &[&str] = &[
    "pln",
    "put",
    "x",
    "y",
    "$x",
    "$y",
    "=",
    "+=",
    ";",
    ";",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    "fn",
    "f",
    "$f",
    "if",
    "else",
    "while",
    "for",
    "in",
    "split",
    "true",
    "false",
    "null",
    "0",
    "1",
    "-1",
    "2.5",
    "-",
    "+",
    "*",
    "/",
    "%",
    "..",
    "==",
    "<",
    "&&",
    "||",
    "??",
    "!",
    "&x",
    "$%x",
    "return",
    "break",
    "continue",
    "try",
    "catch",
    "e",
    "throw",
    "use",
    "\"m\"",
    "\"loop\"",
    "let",
    ",",
    "=",
    "map",
    "len",
    "range",
    "first",
    "sort",
    "join",
    "json.parse",
    "replace",
    "captures",
    "split_re",
    "pad_left",
    "unique",
    "flatten",
    "chunk",
    "group_by",
    "fixed",
    "date.parse",
    "date.format",
    "url.parse",
    "base64.decode",
    "\"\"\"",
    "\"\"\"\"",
    "..",
    "$x[1..]",
    "$x[..-1]",
    "\\d",
    "eval",
    "\"put 1\"",
    "'s'",
    "\"$x\"",
    "x.a",
    "$x.a",
    "$x[0]",
    "x[0]",
    "mod",
    "m",
    "as",
    "from",
    "loop",
    "%r",
    "a:",
    "num",
    "&",
    "&arr",
    ":",
    "|",
    "?",
    "\\",
    "$",
    "${x}",
    "$(1)",
    "//c\n",
    "/*",
    "*/",
];

fn token_soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(TOKENS), 0..40).prop_map(|t| t.join(" "))
}

/// Mostly well-formed programs, built from a small grammar, so the VM (not
/// just the parser) gets exercised.
fn program() -> impl Strategy<Value = String> {
    let atom = prop_oneof![
        Just("1".to_string()),
        Just("-3".to_string()),
        Just("word".to_string()),
        Just("$x".to_string()),
        Just("$y".to_string()),
        Just("[a 1]".to_string()),
        Just("[k = 1]".to_string()),
        Just("(null)".to_string()),
        Just("(true)".to_string()),
        Just("\"s $x\"".to_string()),
        Just("{ put 1; put 2 }".to_string()),
    ];
    let value = atom.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            (
                inner.clone(),
                inner.clone(),
                prop::sample::select(&["+", "-", "*", "/", "%", "==", "<", "..", "&&", "??"][..])
            )
                .prop_map(|(a, b, op)| format!("({a} {op} {b})")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{{ put {a}; put {b} }}")),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| format!("(if {a} {{ put {b} }} else {{ put 0 }})")),
            inner
                .clone()
                .prop_map(|a| format!("(for i in {a} {{ put [$i] }})")),
            inner
                .clone()
                .prop_map(|a| format!("(try {{ put {a} }} catch e {{ put $e.kind }})")),
            inner.clone().prop_map(|a| format!("({{ f {a} }})")),
            inner.clone().prop_map(|a| format!("[{a} {a}]")),
            inner.clone().prop_map(|a| format!("$x[{a}]")),
            inner.prop_map(|a| format!("{{ len {a} }}")),
        ]
    });
    let stmt = value.prop_flat_map(|v| {
        prop_oneof![
            Just(format!("x = {v}")),
            Just(format!("y = {v}")),
            Just(format!("x += {v}")),
            Just(format!("pln {v}")),
            Just(format!("put {v}")),
            Just(format!("while {v} {{ x = 0; break }}")),
            Just(format!("incr &x {v}")),
            Just(format!("x.k = {v}")),
            Just(format!("g = fn (a) {{ put ($a + {v}) }}; pln ($g(1))")),
        ]
    });
    prop::collection::vec(stmt, 1..8).prop_map(|stmts| {
        format!(
            "x = 1; y = [1 2]; fn f (a) {{ put $a }} fn incr (n: &, d) {{ n += $d }}\n{}",
            stmts.join(";\n")
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn token_soup_never_panics(source in token_soup()) {
        run(&source);
    }

    #[test]
    fn generated_programs_never_panic(source in program()) {
        run(&source);
    }

    #[test]
    fn arbitrary_text_never_panics(source in "\\PC{0,100}") {
        run(&source);
    }
}

#[test]
fn known_tricky_programs() {
    for source in [
        "",
        ";;;",
        "{}",
        "x = { break }",
        "fn f () { return } f; f; f",
        "x = [=]; x[0] = 1",
        "x = []; x[-1] = 1",
        "for x in (0..5) { for y in (0..5) { if ($y == 2) { break }; continue } }",
        "try { try { throw a } catch e { throw $e } } catch e { put $e }",
        "use mod loop",
        "eval \"eval \\\"eval 1\\\"\"",
        "fn f (n: num) { put { f ($n + 1) } } f 0",
        "x = [1]; fn g (a: &) { a += $a } g &x; g &x",
        "map [1 2] fn (x) { break }",
        "x = \"a\" * 1000000",
        "x = (0..1000000)",
        "m = mod { n = mod { o = mod { use mod m } } }",
        "throw (fn () {})",
        "put $%x",
        "f = fn () { put $f }; pln ($f())",
    ] {
        run(source);
    }
}

/// Guards the generator itself: if it drifted into producing mostly invalid
/// programs, the VM would silently stop being exercised.
#[test]
fn generated_programs_mostly_compile_and_run() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut ran, mut compile_errors) = (0, 0);
    for _ in 0..300 {
        let source = program().new_tree(&mut runner).unwrap().current();
        let mut rt = Builder::new()
            .limits(tight_limits())
            .output(OutputBuffer::default())
            .build();
        match rt.run("fuzz", &source) {
            Err(e) if matches!(e.kind(), dog4::ErrorKind::Syntax | dog4::ErrorKind::Name) => {
                compile_errors += 1;
            }
            Ok(_) => ran += 1,
            Err(_) => {}
        }
    }
    assert!(
        compile_errors < 15,
        "{compile_errors} of 300 didn't compile"
    );
    assert!(ran > 15, "only {ran} of 300 ran to completion");
}
