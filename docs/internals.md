# dog4 internals

A map of the implementation for people changing it.

```text
source ──parse──▶ AST ──compile──▶ Proto (bytecode) ──run──▶ Value
        syntax/            compile/                  vm/
```

| module | role |
|---|---|
| `syntax/` | hand-written parser (`parser.rs` statements, words, literals; `expr.rs` Pratt expressions), AST, spans, `SourceMap` |
| `compile/` | name resolution and code generation (`compiler.rs`), the instruction set (`bytecode.rs`), `disasm.rs` |
| `vm/` | the interpreter loop (`mod.rs`), calls and overloads (`call.rs`), functions (`function.rs`), cells, references and module globals (`binding.rs`), `for` iteration (`iter.rs`) |
| `value/` | `Value`, the fold and other operators (`ops.rs`), types and coercion (`types.rs`), display, JSON |
| `stdlib/` | builtins, registered with dog4-syntax signatures through `Library` |
| `module/` | `ModuleProvider` and the fs / memory / chain providers |
| `runtime.rs` | the public `Runtime` / `Builder` API |
| `limits.rs`, `error.rs` | sandbox limits and the `Meter`; the single `Error` type |

## Parsing

The parser reads characters directly (there is no token stream) because the
same characters mean different things in different places: in command mode
`1+1` is one word, in expression mode it is an addition. The current mode is
simply which function is running:

* `stmts`/`stmt` decide what a statement is from its first word (keyword,
  `{`, `$`, or a name followed by an assignment operator).
* `value(mode)` reads one command-mode value: a block, literal, `( expr )`,
  control form, lambda, or `word`.
* `word(mode)` collects pieces (text, quotes, `$` forms) until a word
  boundary; several pieces become `ExprKind::Glue`. A plain word that reads as
  a number or bool becomes a literal (`bare_literal`).
* `expr` is a Pratt parser. Bare identifiers are strings unless followed by
  `(` (a call) or, on the same line, by an argument (an embedded command).

Every node carries a byte `Span`; nesting depth is bounded by
`Limits::max_nesting` so hostile input can't overflow the stack.

## Compiling

`compile_module` turns a module body into a `Proto` whose code returns the
module's output. Each function (and lambda) becomes a nested `Proto`.

**Names.** A pre-pass collects the module's top-level names (assignments,
`let`, `fn`, `use … from`) into its `ModuleScope`, so functions can refer to
globals defined later. Inside, a name resolves to, in order: a local slot of
the current function, an upvalue (a local of an enclosing function), a module
global, or a prelude entry. Assignment follows dog3's rule — the nearest
visible variable, otherwise a new local in the current block.

**Glob imports.** `use map` stores the map in a hidden variable named
`*source:offset` (a module global at the top level, otherwise a slot declared
at the start of its block so hoisted functions can capture it). A name with
globs between its use and its declaration compiles to `LoadImported(i)`, whose
`Proto::lookups[i]` lists those hidden variables in the order to check them and
the static fallback (a variable, a prelude entry, or "not defined"). Code with
no glob in scope compiles exactly as before.

**Slots and cells.** Every declaration gets its own slot. When a closure
captures a local, or `&x` is taken, the local is *boxed* into a cell. That is
often discovered after code using the slot was emitted, so `FnState::finish`
patches the function: `LoadLocal(s)` → `LoadCell(c)`, declaring stores →
`DeclCell` (a fresh cell, so each loop iteration has its own variable),
placeholder `Nop`s → `NewCell` (hoisted functions) or `BoxParam`.

**Blocks** compile to `AccBegin … AccEnd`; statements that output end with
`AccPut`. Blocks that can't output anything skip the accumulator.

**Hoisting.** All `fn`s of a block are created at the block's start; several
with the same name are combined with `MakeOverloads`.

**Intrinsics.** `put x` with the builtin `put` compiles to just `x`.
`for i in (a..b)` uses `IterRange` and never builds the array.

## The VM

A stack machine with an 8-byte `Op`. Besides the operand stack it has:

| stack | holds |
|---|---|
| `frames` | function activations: the closure, `ip`, `base` (slot 0's stack index), the frame's cells, and the heights of the other stacks at entry |
| `accs` | block accumulators |
| `iters` | running `for` loops |
| `loops` | loop records (stack heights) for `break`/`continue` |
| `handlers` | `try` handlers (stack heights and the catch address) |

Frames live on the heap, so deep recursion is bounded by `max_call_depth`, not
the Rust stack. `dispatch` caches the current frame's proto, `ip` and `base` in
locals and reloads them after calls and returns.

**Calls.** `call_value` picks the overload (`select`: arity, then exact types,
then convertible types), converts arguments in place (`prepare_args`), then
either pushes a frame (script) or runs the native immediately with a pooled
argument buffer. Script functions get `%rest` as an array; natives get extra
arguments flat.

**Natives calling scripts** (`Vm::call`) run a nested `run(stop)` that returns
when the frame depth drops back to `stop`. Only handlers pushed above `stop`
can catch its errors. `max_native_depth` bounds this recursion.

**Errors.** Ops return `Result`; on error `run` attaches the current op's span,
then either jumps to the innermost catchable handler (restoring stack heights
and pushing the error map) or unwinds to `stop` and returns it with a trace.

**References** are `RefData { target, path }`, where the target is a cell or a
module global. They travel only as call arguments (`Value::Ref`) and are bound
to cells by `BindRef`/`DeclRef`, so scripts never hold one as a value.

**Modules.** `mod name` (`LoadModule`) asks native modules, then the cache, then the provider.
Scripts run in a frame of kind `Module`; when it returns, its `ModuleEnv`'s
`pub` globals (flagged in its `ModuleScope`) become the map, which is cached.
`pub use` globs are merged in after the `pub` names. A loading stack detects
cycles. `mod { … }` literals are compiled separately and run the same way
(uncached).

## Values and memory

`Value` is 16 bytes. Strings, arrays and maps are `Rc` with copy-on-write
(`Rc::make_mut`), so the fold appends in place when a value is uniquely
owned — which is why accumulating in a block or with `+=` is amortised O(1).
`Array`/`Object` drop iteratively, so deeply nested values can't overflow the
stack.

Closures can form `Rc` cycles (through cells and module globals). The VM keeps
weak lists of cells and module environments and clears them when it is
dropped, so a dropped runtime frees everything.

## The sandbox

`Meter` counts fuel (backward jumps and calls cost 1; natives charge for the
data they copy), checks the deadline and the interrupt flag every 4096 units
(and right after blocking natives such as `get`, which also cap their own
timeouts at the time left), and enforces string, collection and output sizes *before* allocating. Regexes
use the linear-time `regex` crate with size limits; JSON parsing is depth
limited by serde. Limit errors can't be caught.

## Testing

| suite | what |
|---|---|
| unit tests in `src/` | values (including property tests of the fold), parser (S-expression snapshots, never-panics properties), signatures, providers, limits |
| `tests/scripts.rs` | golden files: `tests/scripts/**`, the dog3 fixtures in `tests/dog3`, and `examples/`. `BLESS=1` rewrites expectations |
| `tests/docs.rs` | runs every `dog` example in the README and `docs/` |
| `tests/compile.rs` | bytecode shape via disassembly |
| `tests/api.rs` | the embedding API, limits, interrupts, cycles |
| `tests/robustness.rs` | random and generated programs under tight limits must not panic |
| `benches/vm.rs` | criterion benchmarks |
| `fuzz/` | the same robustness harness for `cargo +nightly fuzz run run` |
| `playground/test.mjs` | the WebAssembly build, through the same interface the browser uses |

## The playground

`playground/` compiles dog4 to `wasm32-unknown-unknown` with no JavaScript glue
library: the module exports `dog4_alloc`, `dog4_free`, `dog4_run` and
`dog4_result_len`, and results come back as JSON. dog4 needs no imports from
the host — there is no clock there, so the playground bounds scripts by fuel,
and its random seed comes from JavaScript. `web/worker.js` runs the module off
the main thread; `web/app.js` is the editor (a transparent textarea over
highlighted text). On every push to `main`,
`.github/workflows/playground.yml` builds `web/`, runs `test.mjs` and
publishes it to GitHub Pages.
