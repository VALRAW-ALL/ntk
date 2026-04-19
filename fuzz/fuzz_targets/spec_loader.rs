#![no_main]
//! Fuzz target: spec_loader::apply_rule_file().
//!
//! The spec loader is the newest and broadest attack surface in NTK
//! (RFC-0001 POC): it consumes untrusted YAML rule files from disk
//! and applies them to arbitrary text input. Two orthogonal concerns:
//!
//! 1. Rule file parsing — a malformed YAML should never panic,
//!    only produce a descriptive Err.
//! 2. Apply — given a VALID rule file (the shipped Python ruleset),
//!    arbitrary bytes as input must not panic regardless of encoding,
//!    length, or content.
//!
//! This target covers (2) — apply with the shipped Python ruleset on
//! arbitrary input. Paired with the layer1_filter / layer2_compress
//! fuzz targets, this closes the robustness surface around the
//! compression pipeline.
//!
//! Run (requires nightly + cargo-fuzz):
//!
//!   cd fuzz
//!   cargo +nightly fuzz run spec_loader -- -max_total_time=60

use libfuzzer_sys::fuzz_target;
use ntk::compressor::spec_loader::{apply_rule_file, load_rule_file, RuleFile};
use std::sync::OnceLock;

// Load the Python ruleset once per fuzzer process — parsing YAML per
// iteration would dominate the fuzz loop without exercising the
// actual `apply` surface. OnceLock is stdlib (Rust ≥ 1.70) so no
// additional fuzz-crate dep.
fn python_rules() -> Option<&'static RuleFile> {
    static RULES: OnceLock<Option<RuleFile>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("rules")
                .join("stack_trace")
                .join("python.yaml");
            load_rule_file(&path).ok()
        })
        .as_ref()
}

fuzz_target!(|data: &[u8]| {
    let Some(rules) = python_rules() else {
        return;
    };
    // Hook output arrives as JSON-escaped bytes; simulate that path by
    // decoding losslessly then handing the result to apply_rule_file.
    let s = String::from_utf8_lossy(data);
    // apply_rule_file returns ApplyResult — invariant-rejected rules
    // are fine, actual panics are what we're hunting.
    let _ = apply_rule_file(&s, rules);
});
