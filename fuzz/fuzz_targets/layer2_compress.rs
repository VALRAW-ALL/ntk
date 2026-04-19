#![no_main]
//! Fuzz target: layer2_tokenizer::process().
//!
//! Invariant under test: tiktoken-rs panics on a malformed BPE vocab
//! lookup in older versions; this target confirms that the current
//! cl100k_base instance handles adversarial bytes (binary, invalid
//! UTF-8, null bytes, huge repeats) without panicking or exceeding
//! a reasonable allocation budget.
//!
//! Run (requires nightly + cargo-fuzz):
//!
//!   cargo install cargo-fuzz
//!   cd fuzz
//!   cargo +nightly fuzz run layer2_compress -- -max_total_time=60

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    // L2 returns Result — Err is acceptable (fuzzer asserts no panic).
    let _ = ntk::compressor::layer2_tokenizer::process(&s);
});
