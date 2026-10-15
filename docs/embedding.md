# Embedding dog4

dog4 is a library first. The Rust examples on this page are compiled and run by
`cargo test --doc`.

## Running code

```rust
use dog4::{Runtime, Value};

let mut rt = Runtime::new();
let out = rt.run("main", "x = 20; put ($x + 1)").unwrap();
assert_eq!(out, Value::Int(21));
assert_eq!(out.to_string(), "21");
```

`Runtime::run(name, source)` compiles and runs a script and returns what it
outputs. `name` identifies the script in error messages and is the base for
relative `use` paths. Other entry points:

| method | does |
|---|---|
| `run(name, source)` | runs an entry script, returns its output |
| `import(name)` | loads a module through the provider, returns its map |
| `call(&function, args)` | calls a dog4 function value |
| `eval(code)` | runs code in a persistent session (globals survive between calls) |
| `disassemble(name, source)` | the bytecode listing |
| `interrupt_handle()` | a `Send` handle that stops the running script |

Every one of these calls gets a fresh fuel and time budget.

## Configuring: `Builder`

```rust
use dog4::{Builder, Capability, Limits, OutputBuffer};

let out = OutputBuffer::default();
let mut rt = Builder::new()
    .limits(Limits { fuel: Some(1_000_000), ..Limits::default() })
    .allow(Capability::Net, false)
    .output(out.clone())
    .seed(42)
    .build();
rt.run("main", "println hello").unwrap();
assert_eq!(out.take(), "hello\n");
```

* `Builder::new()` is the trusted configuration: all capabilities, generous
  limits, output to stdout.
* `Builder::sandboxed()` starts from `Limits::sandbox()` and withholds the
  network. Give it an `output` you control.
* `.allow(capability, bool)` grants or withholds `Print` (`print`, `println`),
  `Eval` (`eval`), `Random` (`random`, `shuffle`, `pick`) and `Net` (`get`,
  `post`; needs the `net` feature). Withheld builtins don't exist: using them is
  a compile error ("unknown command").
* `.global(name, value)` adds a name every script sees (or replaces a builtin).
* `.module(name, value)` registers a native module for `use "name"`.
* `.provider(p)` says where other modules come from.
* `.output(w)` is where `print` writes; `OutputBuffer` captures it in memory.
* `.seed(n)` makes `random` reproducible.

## Limits

| field | default | `Limits::sandbox()` |
|---|---|---|
| `fuel` — work units per call (loops and calls cost 1; copying data costs ~1 per 64 bytes) | unlimited | 5,000,000 |
| `timeout` | none | 2 s |
| `max_call_depth` | 100,000 | 512 |
| `max_native_depth` — nested native→script callbacks | 1,000 | 32 |
| `max_str_bytes` | unlimited | 1 MiB |
| `max_collection_len` | unlimited | 100,000 |
| `max_output_bytes` — printed per call | unlimited | 64 KiB |
| `max_nesting` — source nesting depth | 512 | 128 |

Sizes are checked *before* allocating, so `"a" * 1e12` fails immediately. Limit
errors have kind `ErrorKind::Limit(_)` and can't be caught by scripts.

```rust
use std::time::Duration;
use dog4::{Builder, ErrorKind, Limit, Limits};

let mut rt = Builder::new()
    .limits(Limits { timeout: Some(Duration::from_millis(20)), ..Limits::default() })
    .build();
let err = rt.run("main", "while true {}").unwrap_err();
assert_eq!(err.kind(), ErrorKind::Limit(Limit::Timeout));
// The runtime is still usable.
assert_eq!(rt.run("main", "put ok").unwrap().to_string(), "ok");
```

To stop a script from another thread, use `rt.interrupt_handle().trigger()`.

## Native functions and modules

Natives are Rust closures with a signature written in dog4 syntax. Arguments
arrive already converted to the declared types; `%rest` arguments follow the
positional ones.

```rust
use std::cell::RefCell;
use std::rc::Rc;
use dog4::{Builder, Library, Value};

let sent = Rc::new(RefCell::new(Vec::new()));
let log = sent.clone();

let mut telegram = Library::new();
telegram.def("send", "chat: num, %text", move |_vm, args| {
    let words: Vec<String> = args[1..].iter().map(|v| v.to_string()).collect();
    log.borrow_mut().push(format!("{} <- {}", args[0], words.join(" ")));
    Ok(Value::Null)
});
// Defining a name again adds an overload.
telegram.def("send", "text: str", |_, _| Ok(Value::Null));

let mut rt = Builder::new().module("telegram", telegram.into_value()).build();
rt.run("bot", r#"use "telegram"; telegram.send "42" hello world"#).unwrap();
assert_eq!(*sent.borrow(), ["42 <- hello world"]);
```

A single native can also be made with `Value::native(name, params, f)` and
handed to scripts with `Builder::global`.

Natives receive `&mut Vm`, which can call back into scripts (`vm.call(&f,
args)`), print (`vm.print(text)`), charge for work (`vm.meter()`) and use the
runtime's random number generator (`vm.rng()`):

```rust
use dog4::{Builder, Value};

let twice = Value::native("twice", "f: fn, x", |vm, args| {
    let f = args[0].clone();
    let once = vm.call(&f, vec![args[1].clone()])?;
    vm.call(&f, vec![once])
});
let mut rt = Builder::new().global("twice", twice).build();
let out = rt.run("main", "put { twice fn (x) { put ($x * 10) } 3 }").unwrap();
assert_eq!(out.to_string(), "300");
```

Errors are returned as `dog4::Error`: `Error::type_error(..)`,
`Error::value_error(..)`, `Error::thrown(value)`, or `Error::new(kind, ..)`.

## Module providers

The runtime doesn't know about files. For `use "name"` it asks the native
modules registered with `.module`, then the provider:

```rust,ignore
pub trait ModuleProvider {
    /// Turn the name written in `use` into a canonical id. `from` is the id of the
    /// importing module. The default resolves `./x` and `../x` relative to `from`.
    fn resolve(&self, name: &str, from: Option<&str>) -> Result<String, ProviderError>;
    /// Load the module with that id: source code, or a ready-made map.
    fn load(&self, id: &str) -> Result<ModuleSource, ProviderError>;
}
```

Ids are the cache keys: each module runs once per runtime. Built-in providers:

* `FsProvider::new(root)` — `"lib/x"` is `<root>/lib/x.dog`; paths can't escape `root`.
* `MemoryProvider` — modules held in a map (handy for tests and bots).
* `ChainProvider` — tries several providers in order.

Anything else (a database, a key-value store) is a small `impl`:

```rust
use std::collections::HashMap;
use dog4::{Builder, ModuleProvider, ModuleSource, ProviderError};

/// Pretend this is a database table of user scripts.
struct Table(HashMap<String, String>);

impl ModuleProvider for Table {
    fn load(&self, id: &str) -> Result<ModuleSource, ProviderError> {
        self.0
            .get(id)
            .map(|source| ModuleSource::Script(source.clone()))
            .ok_or(ProviderError::NotFound)
    }
}

let table = Table(HashMap::from([(
    "users/ana/greet".to_string(),
    "pub fn hi (name: str) { put \"hi $name\" }".to_string(),
)]));
let mut rt = Builder::new().provider(table).build();
let out = rt.run("users/ana/main", r#"use "./greet"; greet.hi Bob"#).unwrap();
assert_eq!(out.to_string(), "hi Bob");
```

## Calling script functions from Rust

```rust
use dog4::{MemoryProvider, Runtime, Value};

let modules = MemoryProvider::new().with("math", "pub fn area (w: num, h: num) { put ($w * $h) }");
let mut rt = Runtime::builder().provider(modules).build();
let math = rt.import("math").unwrap();
let Value::Map(map) = &math else { unreachable!() };
let area = map.get("area").unwrap().clone();
// Arguments are converted just like in scripts.
let result = rt.call(&area, vec![Value::str("3"), Value::Int(4)]).unwrap();
assert_eq!(result, Value::Int(12));
```

Values convert to and from JSON with `dog4::value::to_json` and
`dog4::value::from_json`.

## Errors

`Error` implements `Display` with a full report:

```text
error: `double` expects a num for `x`, got str "abc"
 --> main:2:1
  |
2 | double abc;
  | ^^^^^^^^^^
  in some_caller (main:5:3)
```

It also exposes `kind()`, `message()`, `location()` (file, line, column),
`trace()` and `value()` (what was thrown).

## Threads

A `Runtime` uses reference counting internally and is not `Send`: create one
per thread (or per task, inside `spawn_blocking`). Values returned from a
runtime are also not `Send`; convert them to JSON or text to move them. The
`Interrupt` handle *is* `Send`, so a watchdog thread can stop a script.

Dropping a runtime frees everything its scripts allocated, including
reference cycles created by closures.

## A Telegram bot, sketched

```rust
use dog4::{Builder, Library, Limits, MemoryProvider, OutputBuffer, Value};

/// Runs one message's worth of user code and returns what to reply.
fn handle(code: &str, user_modules: MemoryProvider) -> String {
    let out = OutputBuffer::default();
    let mut api = Library::new();
    api.def("now", "", |_, _| Ok(Value::Int(1_700_000_000)));
    let mut rt = Builder::sandboxed()
        .limits(Limits { fuel: Some(200_000), ..Limits::sandbox() })
        .output(out.clone())
        .module("bot", api.into_value())
        .provider(user_modules)
        .build();
    match rt.run("message", code) {
        Ok(value) => format!("{}{value}", out.take()),
        Err(e) => format!("{}{e}", out.take()),
    }
}

let modules = MemoryProvider::new().with("dice", "pub fn d (n: num) { put ($n * 0 + 4) }");
assert_eq!(handle(r#"use "dice"; pln rolled { dice.d 6 }"#, modules), "rolled 4\n");
let reply = handle("while true {}", MemoryProvider::new());
assert!(reply.starts_with("error: ran out of fuel"));
```
