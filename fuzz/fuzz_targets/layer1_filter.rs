#![no_main]
//! Fuzz target: layer1_filter::filter().
//!
//! Invariant under test: for ANY byte sequence (interpreted as UTF-8 via
//! lossy conversion, just as the hook does with real command output),
//! the function returns an Ok-equivalent value — it never panics, never
//! overflows, never unbounded-allocates.
//!
//! Run (requires nightly + cargo-fuzz):
//!
//!   cargo install cargo-fuzz
//!   cd fuzz
//!   cargo +nightly fuzz run layer1_filter -- -max_total_time=60

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Real-world input arrives as command stdout/stderr bytes. The hook
    // feeds the daemon via JSON, which already produces a String — so we
    // exercise the same lossy-UTF-8 path here.
    let s = String::from_utf8_lossy(data);
    let _ = ntk::compressor::layer1_filter::filter(&s);
});
