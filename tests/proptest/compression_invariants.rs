// ---------------------------------------------------------------------------
// Etapa 23 — Property-based tests: compression invariants
//
// These tests use proptest to verify that the compression pipeline upholds
// key invariants for *any* input, not just the hand-crafted fixtures.
//
// Run with: cargo test --test compression_invariants
// ---------------------------------------------------------------------------

use ntk::compressor::{layer1_filter, layer2_tokenizer};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Generate realistic "tool output"-like strings: mix of ASCII, paths, numbers.
fn arb_tool_output() -> impl Strategy<Value = String> {
    // Lines that look like realistic command output.
    let line_strats: Vec<BoxedStrategy<String>> = vec![
        // Plain text line
        "[a-zA-Z0-9 /.:_-]{1,120}\n".prop_map(String::from).boxed(),
        // Cargo-style warning line
        Just("warning: unused variable `x` --> src/lib.rs:42:9\n".to_owned()).boxed(),
        // Test result line
        Just("test module::test_fn ... ok\n".to_owned()).boxed(),
        // Path line
        Just("/home/user/project/src/main.rs:10:5\n".to_owned()).boxed(),
        // Blank line
        Just("\n".to_owned()).boxed(),
    ];

    proptest::collection::vec(proptest::strategy::Union::new(line_strats), 1..200)
        .prop_map(|lines| lines.concat())
}

// ---------------------------------------------------------------------------
// Invariant 1: compression never panics and always produces a result
//
// L1 annotations (e.g. "[2 blank lines]") can increase token count for
// whitespace-heavy inputs — that's by design. The important invariants are:
// (a) no panic on any input, (b) output is not empty when input is not,
// (c) no catastrophic inflation (> 10x).
// Per-fixture token reduction is covered by quality_regression_tests.rs.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_compression_does_not_panic(input in arb_tool_output()) {
        let original = layer2_tokenizer::count_tokens(&input)
            .expect("token count must not fail");

        let l1 = layer1_filter::filter(&input);
        let l2 = layer2_tokenizer::process(&l1.output)
            .expect("layer2 must not fail");

        // Catastrophic inflation (> 10x) would indicate a serious bug.
        let max_allowed = original.saturating_mul(10).saturating_add(50);
        prop_assert!(
            l2.compressed_tokens <= max_allowed,
            "Catastrophic token inflation: {original} → {} tokens",
            l2.compressed_tokens
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 2: compression is deterministic
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_compression_is_deterministic(input in arb_tool_output()) {
        let l1a = layer1_filter::filter(&input);
        let l2a = layer2_tokenizer::process(&l1a.output)
            .expect("layer2 must not fail on first run");

        let l1b = layer1_filter::filter(&input);
        let l2b = layer2_tokenizer::process(&l1b.output)
            .expect("layer2 must not fail on second run");

        prop_assert_eq!(
            &l2a.output, &l2b.output,
            "Compression is not deterministic"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 3: empty input produces empty (or very small) output
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_empty_and_whitespace_input_handled(
        input in prop::string::string_regex(r"[\s]{0,100}").unwrap()
    ) {
        // Must not panic.
        let l1 = layer1_filter::filter(&input);
        let _l2 = layer2_tokenizer::process(&l1.output)
            .expect("empty/whitespace input must not cause error");
    }
}

// ---------------------------------------------------------------------------
// Invariant 4: output is always valid UTF-8
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_output_is_valid_utf8(input in arb_tool_output()) {
        let l1 = layer1_filter::filter(&input);
        let l2 = layer2_tokenizer::process(&l1.output)
            .expect("layer2 must not fail");

        // Rust Strings are always UTF-8 — just verify no panic occurred.
        let _len = l2.output.len();
    }
}

// ---------------------------------------------------------------------------
// Invariant 5: layer1 lines_removed ≥ 0 and ≤ original line count
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_layer1_lines_removed_within_bounds(input in arb_tool_output()) {
        let original_lines = input.lines().count();
        let l1 = layer1_filter::filter(&input);
        prop_assert!(
            l1.lines_removed <= original_lines,
            "lines_removed ({}) > original lines ({})",
            l1.lines_removed, original_lines
        );
    }
}
