# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`dog4` is an esoteric, Bash-flavoured scripting language in Rust, the successor of dog3, built to be embedded and sandboxed (its main host is a Telegram bot running untrusted user code). Every statement *outputs* into its enclosing block and a block's value is the fold (`+`) of those outputs: numbers add, text/arrays concatenate, maps merge. `docs/language.md` is the precise reference; `docs/internals.md` explains the implementation; `docs/embedding.md` the Rust API.

## Commands

```sh
cargo build
cargo run -- file.dog            # run a script (FsProvider rooted at the file's directory)
cargo run -- -e 'pln hi'         # run a snippet; no args = REPL; --disasm prints bytecode
cargo test                       # everything (unit, golden scripts, docs, api, compile, robustness, doctests)
cargo test --test scripts -- functions/closures   # one golden script (names are paths without .dog)
BLESS=1 cargo test --test scripts                  # rewrite .out/.err expectations — then READ the diffs
cargo test --test docs           # runs every ```dog block in README.md, MIGRATING.md, docs/*.md
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench vm           # criterion benchmarks
playground/build.sh              # build the browser playground (dog4 → wasm32) into playground/web
node playground/test.mjs         # smoke-test the built .wasm
playground/deploy.sh             # build and commit it to the gh-pages branch, then `git push origin gh-pages`
```

The playground (`playground/`) is a separate crate (its own `[workspace]`) exporting a C-style API to JavaScript; `web/` is plain HTML/JS with a Web Worker and its own highlighter (`highlight.js`). Its examples menu is generated from `examples/*.dog`.

## Architecture

source → `syntax/` (hand-written, scannerless parser; command mode vs expression mode is decided by which function is running) → `compile/` (resolves names to slots / cells / upvalues / globals / prelude, emits `Op`s, patches captured locals into cells in `FnState::finish`) → `vm/` (stack machine; extra stacks for block accumulators, iterators, loop records, try handlers; heap frames). Builtins live in `stdlib/`, registered through `Library::def(name, "dog4 param list", closure)`; natives get `%rest` args flat, scripts get an array. Modules come from a host `ModuleProvider` (`module/`); `runtime.rs` is the public API.

## Gotchas when writing dog4 code (tests, docs, examples)

- A bare block or control statement (`{ … }`, `if`, `for`, `while`) used as a *statement* has its value discarded — `{ if c { pln x } }` prints nothing. Use `put if …`, or `x = for …` (the right side of `=` is an expression).
- Newlines don't separate statements; `;` is required after commands and assignments.
- Bare words are text, except exact numbers/bools (`5`, `true`; not `007`, not `null`). In expressions, bare identifiers are strings unless called (`f(x)`) or followed by arguments on the same line (`(len $x)`).
- `$name.txt` is a field access; write `${name}.txt`.
- Always run new dog4 snippets; the doc and golden tests will catch drift. After `BLESS=1`, review every changed expectation — never bless blindly.

## Conventions

- Every language change needs: a golden script in `tests/scripts/<area>/`, docs in `docs/language.md` (with a runnable example), and a note in `MIGRATING.md` if dog3 behaviour changes.
- The dog3 fixtures (`tests/dog3/*.out`) are dog3's real output and are never rewritten; intended differences go in `.dog4.out`/`.dog4.err` plus `tests/dog3/EXPECTED_DIFFERENCES.md`.
- Untrusted input must never panic: no `unwrap` on user-reachable paths, size checks before allocating (`Meter::str_len` / `coll_len`), and new natives that loop or copy should charge fuel.
- The approved design lives in the plan the project started from; ask before changing core semantics (fold rules, scoping rule, discard rule, bare-literal rule).
