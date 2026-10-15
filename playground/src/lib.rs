//! The dog4 playground's WebAssembly module.
//!
//! The interface is plain C-style exports so the page needs no glue library:
//! JavaScript allocates a buffer with [`dog4_alloc`], copies UTF-8 source into
//! it, calls [`dog4_run`], and reads the JSON result from the pointer it
//! returns ([`dog4_result_len`] bytes long).

use std::cell::RefCell;

use dog4::{Builder, Capability, Limits, MemoryProvider, OutputBuffer, Runtime};
use serde_json::json;

thread_local! {
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Modules that `use` can load in the playground.
const MODULES: &[(&str, &str)] = &[
    (
        "greet",
        "greeting = Hello;\npub fn hi (name: str) { pln $greeting $name }\npub loaded = yes",
    ),
    (
        "shapes",
        "pub fn square (side: num) { put ($side * $side) }\npub fn circle (r: num) { put (3.14159 * $r * $r) }",
    ),
];

fn runtime(seed: u64, out: &OutputBuffer) -> Runtime {
    let mut modules = MemoryProvider::new();
    for (name, source) in MODULES {
        modules.insert(name, source);
    }
    let limits = Limits {
        fuel: Some(20_000_000),
        // There is no clock in plain WebAssembly; fuel bounds running time.
        timeout: None,
        max_call_depth: 5_000,
        max_native_depth: 64,
        max_str_bytes: 16 << 20,
        max_collection_len: 1_000_000,
        max_output_bytes: Some(1 << 20),
        max_nesting: 256,
    };
    Builder::new()
        .limits(limits)
        .allow(Capability::Net, false)
        .output(out.clone())
        .provider(modules)
        .seed(seed)
        .build()
}

fn execute(source: &str, seed: u64, disassemble: bool) -> serde_json::Value {
    let out = OutputBuffer::default();
    let mut rt = runtime(seed, &out);
    let result = if disassemble {
        rt.disassemble("playground", source)
            .map(|listing| dog4::Value::str(listing))
    } else {
        rt.run("playground", source)
    };
    match result {
        Ok(value) => json!({
            "ok": true,
            "printed": out.take(),
            "output": value.to_string(),
        }),
        Err(err) => {
            let location = err.location();
            json!({
                "ok": false,
                "printed": out.take(),
                "error": err.to_string(),
                "kind": err.kind().name(),
                "line": location.map(|l| l.line),
                "col": location.map(|l| l.col),
            })
        }
    }
}

/// Allocates `len` bytes for the caller to fill.
#[unsafe(no_mangle)]
pub extern "C" fn dog4_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Frees a buffer returned by [`dog4_alloc`].
///
/// # Safety
///
/// `ptr` must come from `dog4_alloc(len)` and not have been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dog4_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
}

/// Runs (or, with `mode == 1`, disassembles) the UTF-8 source at `ptr`, and
/// returns a pointer to the JSON result.
///
/// # Safety
///
/// `ptr` must point to `len` initialized bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dog4_run(ptr: *const u8, len: usize, seed: u32, mode: u32) -> *const u8 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(bytes);
    let result = execute(&source, u64::from(seed), mode == 1).to_string();
    RESULT.with(|cell| {
        let mut buffer = cell.borrow_mut();
        *buffer = result.into_bytes();
        buffer.as_ptr()
    })
}

/// The length of the last result returned by [`dog4_run`].
#[unsafe(no_mangle)]
pub extern "C" fn dog4_result_len() -> usize {
    RESULT.with(|cell| cell.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_reports() {
        let ok = execute("println hi; put [a 1]", 1, false);
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["printed"], "hi\n");
        assert_eq!(ok["output"], "[a 1]");
        let err = execute("pln ok;\npln $nope", 1, false);
        assert_eq!(err["ok"], false);
        assert_eq!(err["line"], 2);
        assert_eq!(err["kind"], "name");
        let modules = execute("use \"greet\"; greet.hi Bob; put { shapes_ok }", 1, false);
        assert_eq!(modules["ok"], false);
        let listing = execute("put 1", 1, true);
        assert!(listing["output"].as_str().unwrap().contains("Const"));
        let fuel = execute("while true {}", 1, false);
        assert_eq!(fuel["kind"], "limit");
    }
}
