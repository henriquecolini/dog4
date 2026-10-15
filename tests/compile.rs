//! Compiler tests, via disassembly listings.

use dog4::{ErrorKind, Runtime};

fn disasm(source: &str) -> String {
    Runtime::new()
        .disassemble("t", source)
        .unwrap_or_else(|e| panic!("compile error: {e}"))
}

fn compile_error(source: &str) -> dog4::Error {
    Runtime::new()
        .disassemble("t", source)
        .expect_err("expected a compile error")
}

#[test]
fn listing_shape() {
    let listing = disasm("x = 1 + 2; pln $x");
    let expected = "\
fn <main> (slots 0, cells 0, captures [])
     0  AccBegin
     1  Const(0)  ; 1
     2  Const(1)  ; 2
     3  Add
     4  StoreGlobal(0)
     5  LoadPrelude(1)
     6  LoadGlobal(0)
     7  Call(1)
     8  AccPut
     9  AccEnd
    10  Return
";
    assert_eq!(listing, expected);
}

#[test]
fn plain_locals_use_slots() {
    let listing = disasm("fn f (a) { b = $a; put $b }");
    assert!(listing.contains("LoadLocal(0)"));
    assert!(listing.contains("StoreLocal(1)"));
    assert!(!listing.contains("Cell"));
}

#[test]
fn captured_locals_are_boxed() {
    let listing = disasm("fn f () { n = 0; put fn () { n += 1 } }");
    assert!(listing.contains("DeclCell(0)"), "{listing}");
    assert!(listing.contains("captures [Cell(0)]"), "{listing}");
    assert!(listing.contains("Update { var: Upval(0)"), "{listing}");
    assert!(!listing.contains("StoreLocal(0)"), "{listing}");
}

#[test]
fn captured_parameters_are_boxed_on_entry() {
    let listing = disasm("fn adder (n) { put fn (x) { put ($x + $n) } }");
    assert!(
        listing.contains("BoxParam { slot: 0, cell: 0 }"),
        "{listing}"
    );
}

#[test]
fn reference_parameters_bind_cells() {
    let listing = disasm("fn incr (n: &num) { n += 1 }");
    assert!(
        listing.contains("BindRef { slot: 0, cell: 0 }"),
        "{listing}"
    );
    assert!(listing.contains("Update { var: Cell(0)"), "{listing}");
}

#[test]
fn taking_a_reference_boxes_the_local() {
    let listing = disasm("fn f () { x = 1; g &x; put $x } fn g (r: &) {}");
    assert!(listing.contains("DeclCell(0)"), "{listing}");
    assert!(
        listing.contains("MakeRef { var: Cell(0), keys: 0 }"),
        "{listing}"
    );
    assert!(listing.contains("LoadCell(0)"), "{listing}");
}

#[test]
fn hoisted_functions_that_capture_each_other_get_cells_first() {
    let listing = disasm("fn outer () { fn a () { b } fn b () { a } }");
    assert!(listing.contains("NewCell(0)"), "{listing}");
    assert!(listing.contains("NewCell(1)"), "{listing}");
}

#[test]
fn overloads_are_grouped() {
    let listing = disasm("fn f (a) {} fn f (a, b) {}");
    assert!(listing.contains("MakeOverloads(2)"), "{listing}");
}

#[test]
fn shadowed_prelude_names_resolve_to_globals() {
    let listing = disasm("fn len (x) { pln 1 } len a");
    // Only `pln` (inside the function) comes from the prelude; `len` is the global.
    assert_eq!(listing.matches("LoadPrelude").count(), 1, "{listing}");
    assert!(listing.contains("LoadGlobal(0)"), "{listing}");
}

#[test]
fn builtin_put_of_one_value_is_inlined() {
    let listing = disasm("put 1; put; put a b");
    // `put 1` is just `1`, `put` is `null`; only the two-argument form calls.
    assert_eq!(listing.matches("Call(").count(), 1, "{listing}");
    assert!(listing.contains("Null"), "{listing}");
}

#[test]
fn shadowed_put_is_not_inlined() {
    let listing = disasm("fn put (x) { pln $x } put 1");
    assert!(listing.contains("Call(1)"), "{listing}");
}

#[test]
fn string_constants_are_shared() {
    let listing = disasm("pln a a a");
    assert_eq!(listing.matches("Const(0)").count(), 3, "{listing}");
}

#[test]
fn static_name_errors() {
    let e = compile_error("pln $nope");
    assert_eq!(e.kind(), ErrorKind::Name);
    assert!(e.message().contains("`nope` is not defined"));
    let e = compile_error("nope 1");
    assert!(e.message().contains("unknown command `nope`"));
    let e = compile_error("x += 1; fn f () { y += 1 }");
    assert!(
        e.message().contains("`y` is not defined"),
        "{}",
        e.message()
    );
}

#[test]
fn compile_errors_have_locations() {
    let e = compile_error("pln ok;\npln $nope");
    let loc = e.location().expect("resolved location");
    assert_eq!((loc.line, loc.col), (2, 5));
}

#[test]
fn break_and_continue_need_a_loop_in_the_same_function() {
    assert!(compile_error("break").message().contains("outside a loop"));
    let e = compile_error("for x in [1] { fn f () { continue } }");
    assert!(e.message().contains("outside a loop"));
}
