//! Tests of the embedding API.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dog4::{
    Builder, Capability, ChainProvider, ErrorKind, Library, Limit, Limits, MemoryProvider,
    OutputBuffer, Runtime, Value,
};

fn run(rt: &mut Runtime, src: &str) -> String {
    match rt.run("main", src) {
        Ok(v) => v.to_string(),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn run_returns_the_output_and_print_goes_to_the_sink() {
    let out = OutputBuffer::default();
    let mut rt = Runtime::builder().output(out.clone()).build();
    assert_eq!(run(&mut rt, "println side; put result"), "result");
    assert_eq!(out.take(), "side\n");
}

#[test]
fn values_keep_their_types() {
    let mut rt = Runtime::new();
    let v = rt
        .run("main", "put [n = 1, l = [a (2.5)], ok = true]")
        .unwrap();
    let json = dog4::value::to_json(&v).unwrap();
    assert_eq!(json.to_string(), r#"{"n":1,"l":["a",2.5],"ok":true}"#);
}

#[test]
fn import_and_call_module_functions() {
    let provider = MemoryProvider::new().with(
        "greet",
        "fn hello (name: str) { put \"hi $name\" } fn add (a: num, b: num) { put ($a + $b) }",
    );
    let mut rt = Runtime::builder().provider(provider).build();
    let module = rt.import("greet").unwrap();
    let Value::Map(map) = &module else {
        panic!("module should be a map")
    };
    let hello = map.get("hello").unwrap().clone();
    assert_eq!(
        rt.call(&hello, vec![Value::str("Ana")])
            .unwrap()
            .to_string(),
        "hi Ana"
    );
    let add = map.get("add").unwrap().clone();
    // Arguments are coerced exactly as for script calls.
    let sum = rt.call(&add, vec![Value::str("2"), Value::Int(3)]).unwrap();
    assert_eq!(sum, Value::Int(5));
    let err = rt
        .call(&add, vec![Value::str("x"), Value::Int(3)])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Type);
}

#[test]
fn native_modules_with_host_state() {
    let sent: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = sent.clone();
    let mut telegram = Library::new();
    telegram.def("send", "chat: num, %text", move |_, args| {
        let text: Vec<String> = args[1..].iter().map(|v| v.to_string()).collect();
        log.borrow_mut()
            .push(format!("{}: {}", args[0], text.join(" ")));
        Ok(Value::Null)
    });
    let mut rt = Runtime::builder()
        .module("telegram", telegram.into_value())
        .build();
    run(
        &mut rt,
        r#"use "telegram"; telegram.send 42 hello there; telegram.send "7" bye"#,
    );
    assert_eq!(*sent.borrow(), ["42: hello there", "7: bye"]);
}

#[test]
fn host_globals_can_add_and_override_names() {
    let mut rt = Runtime::builder()
        .global("answer", Value::Int(42))
        .global(
            "put",
            Value::native("put", "%args", |_, _| Ok(Value::str("overridden"))),
        )
        .build();
    assert_eq!(run(&mut rt, "put $answer"), "overridden");
    let mut rt = Runtime::builder().global("answer", Value::Int(42)).build();
    assert_eq!(run(&mut rt, "put ($answer + 1)"), "43");
}

#[test]
fn natives_can_call_back_into_scripts() {
    let twice = Value::native("twice", "f: fn, x", |vm, args| {
        let f = args[0].clone();
        let once = vm.call(&f, vec![args[1].clone()])?;
        vm.call(&f, vec![once])
    });
    let mut rt = Runtime::builder().global("twice", twice).build();
    assert_eq!(
        run(&mut rt, "put { twice fn (x) { put ($x * 3) } 2 }"),
        "18"
    );
}

#[test]
fn eval_sessions_remember_globals() {
    let mut rt = Runtime::new();
    rt.eval("x = 41").unwrap();
    rt.eval("fn inc (n) { put ($n + 1) }").unwrap();
    assert_eq!(rt.eval("inc $x").unwrap(), Value::Int(42));
}

#[test]
fn modules_are_cached_across_runs() {
    let provider = MemoryProvider::new().with("counter", "n = 0; fn next () { n += 1; put $n }");
    let mut rt = Runtime::builder().provider(provider).build();
    assert_eq!(run(&mut rt, r#"use "counter"; counter.next"#), "1");
    assert_eq!(run(&mut rt, r#"use "counter"; counter.next"#), "2");
}

#[test]
fn chain_provider_and_relative_names() {
    let first = MemoryProvider::new().with("app/main", r#"use "./util"; put { util.twice 4 }"#);
    let second = MemoryProvider::new().with("app/util", "fn twice (x: num) { put ($x * 2) }");
    let mut rt = Runtime::builder()
        .provider(ChainProvider::new().with(first).with(second))
        .build();
    // `./util` resolves relative to `app/main`, then falls through to the second provider.
    let err = rt.import("app/main");
    assert!(err.is_ok(), "{}", err.unwrap_err());
}

#[test]
fn every_call_gets_a_fresh_budget() {
    let limits = Limits {
        fuel: Some(1_000),
        ..Limits::default()
    };
    let mut rt = Runtime::builder().limits(limits).build();
    let err = rt.run("main", "while true {}").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Limit(Limit::Fuel));
    assert_eq!(run(&mut rt, "put ok"), "ok");
}

#[test]
fn interrupt_from_another_thread() {
    let mut rt = Runtime::new();
    let handle = rt.interrupt_handle();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        handle.trigger();
    });
    let started = Instant::now();
    let err = rt.run("main", "while true {}").unwrap_err();
    stopper.join().unwrap();
    assert_eq!(err.kind(), ErrorKind::Limit(Limit::Interrupted));
    assert!(started.elapsed() < Duration::from_secs(5));
    // The runtime is still usable afterwards.
    assert_eq!(run(&mut rt, "put again"), "again");
}

#[test]
fn timeout() {
    let limits = Limits {
        timeout: Some(Duration::from_millis(50)),
        ..Limits::default()
    };
    let mut rt = Runtime::builder().limits(limits).build();
    let err = rt.run("main", "while true {}").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Limit(Limit::Timeout));
}

#[test]
fn errors_carry_kind_location_trace_and_value() {
    let mut rt = Runtime::new();
    let err = rt
        .run("main", "fn f () { throw [code = 7] }\nfn g () { f }\ng")
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Thrown);
    let Value::Map(value) = err.value() else {
        panic!("thrown value")
    };
    assert_eq!(value.get("code"), Some(&Value::Int(7)));
    let loc = err.location().unwrap();
    assert_eq!((loc.line, loc.col, &*loc.file), (1, 11, "main"));
    let names: Vec<&str> = err.trace().iter().map(|t| &*t.function).collect();
    assert_eq!(names, ["f", "g"]);
    let text = err.to_string();
    assert!(
        text.starts_with("error: {\"code\": 7}\n --> main:1:11"),
        "{text}"
    );
}

#[test]
fn sandboxed_builder_withholds_the_network() {
    let mut rt = Builder::sandboxed().output(OutputBuffer::default()).build();
    let err = rt.run("main", "get https://example.com").unwrap_err();
    assert!(err.message().contains("unknown command `get`"));
    let mut rt = Runtime::builder().allow(Capability::Print, false).build();
    assert!(rt.run("main", "println hi").is_err());
}

#[test]
fn seeded_randomness_is_reproducible() {
    let roll = |seed| {
        let mut rt = Runtime::builder().seed(seed).build();
        run(&mut rt, "put (for i in (0..10) { put [{ random 1000 }] })")
    };
    assert_eq!(roll(7), roll(7));
    assert_ne!(roll(7), roll(8));
}

#[test]
fn dropping_a_runtime_frees_reference_cycles() {
    // `probe` is a host function value; the script stores it inside a closure
    // that captures itself (a cycle through a cell). When the runtime is
    // dropped, the cycle must be broken and our probe uniquely owned again.
    let probe = Value::native("probe", "", |_, _| Ok(Value::Null));
    let Value::Func(probe_rc) = &probe else {
        unreachable!()
    };
    let probe_rc = probe_rc.clone();
    {
        let mut rt = Runtime::builder().global("probe", probe.clone()).build();
        run(
            &mut rt,
            "fn make () { keep = $probe; f = fn () { $f; $keep }; put $f } \
             g = { make }; m = [self = $g]; m.again = $m",
        );
        assert!(Rc::strong_count(&probe_rc) > 2);
    }
    drop(probe);
    assert_eq!(Rc::strong_count(&probe_rc), 1);
}

#[test]
fn disassembly_is_available() {
    let listing = Runtime::new().disassemble("main", "put 1").unwrap();
    assert!(listing.contains("Const(0)  ; 1"));
}
