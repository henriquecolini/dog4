//! Parses, compiles and runs arbitrary input under tight limits.
//! Run with `cargo +nightly fuzz run run` from the repository root.

#![no_main]

use std::time::Duration;

use dog4::{Builder, Limits, OutputBuffer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else { return };
    let limits = Limits {
        fuel: Some(20_000),
        timeout: Some(Duration::from_secs(1)),
        max_call_depth: 64,
        max_native_depth: 8,
        max_str_bytes: 10_000,
        max_collection_len: 10_000,
        max_output_bytes: Some(10_000),
        max_nesting: 32,
    };
    let mut rt = Builder::new().limits(limits).output(OutputBuffer::default()).build();
    if let Err(e) = rt.run("fuzz", source) {
        let _ = e.to_string();
    }
});
