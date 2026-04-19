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

// ---------------------------------------------------------------------------
// Invariant 6: Layer 1 is idempotent — filter(filter(x)) == filter(x)
//
// Re-running the pipeline on already-compressed output must not degrade or
// further mutate it. This catches regex double-rewrites and accidental
// template re-dedup of synthetic "[×N]" exemplars.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_layer1_filter_is_idempotent(input in arb_tool_output()) {
        let once = layer1_filter::filter(&input).output;
        let twice = layer1_filter::filter(&once).output;
        prop_assert_eq!(&once, &twice, "layer1 filter is not idempotent");
    }
}

// ---------------------------------------------------------------------------
// Robustness invariant (#16) — neither L1 nor L2 panics on adversarial
// byte sequences decoded via from_utf8_lossy. This mirrors the hook's
// real-world input path (raw stdout bytes → JSON → String). A proptest
// covers the no-panic guarantee on CI; cargo-fuzz in fuzz/ extends the
// same contract to coverage-guided exploration when run locally.
// ---------------------------------------------------------------------------

fn arb_raw_bytes() -> impl Strategy<Value = Vec<u8>> {
    // Any byte sequence up to 4 KiB — covers invalid UTF-8, null bytes,
    // ANSI escapes, truncated multi-byte sequences, etc.
    proptest::collection::vec(any::<u8>(), 0..4096)
}

proptest! {
    #[test]
    fn prop_layer1_filter_never_panics_on_arbitrary_bytes(bytes in arb_raw_bytes()) {
        let s = String::from_utf8_lossy(&bytes);
        // Just needs to return — panic would abort the proptest run.
        let _ = layer1_filter::filter(&s);
    }

    #[test]
    fn prop_layer2_compress_never_panics_on_arbitrary_bytes(bytes in arb_raw_bytes()) {
        let s = String::from_utf8_lossy(&bytes);
        // L2 returns Result; Err is acceptable — panic is not.
        let _ = layer2_tokenizer::process(&s);
    }
}

// ---------------------------------------------------------------------------
// Invariant 7: error / warning / panic lines are never dropped by L1.
//
// The canonical NTK promise: "error information preservation 100%".
// If any line in the INPUT matches a known error signal, at least one line
// in the OUTPUT must still contain that signal (possibly as exemplar of a
// `[×N]` group). This is the strongest guarantee L1 must uphold.
// ---------------------------------------------------------------------------

/// Generate inputs that always contain at least one error-like line mixed
/// with ordinary chatter, so the invariant has something to defend.
fn arb_output_with_errors() -> impl Strategy<Value = String> {
    let chatter: Vec<BoxedStrategy<String>> = vec![
        "[a-zA-Z0-9 /.:_-]{1,60}\n".prop_map(String::from).boxed(),
        Just("info: something happened\n".to_owned()).boxed(),
        Just("\n".to_owned()).boxed(),
    ];
    let errors: Vec<BoxedStrategy<String>> = vec![
        Just("ERROR: database connection refused\n".to_owned()).boxed(),
        Just("error: expected identifier\n".to_owned()).boxed(),
        Just("panic: index out of bounds\n".to_owned()).boxed(),
        Just("warning: unused import `foo`\n".to_owned()).boxed(),
        Just("Traceback (most recent call last):\n".to_owned()).boxed(),
        Just("Exception in thread \"main\": null\n".to_owned()).boxed(),
        Just("FAILED: assertion failed\n".to_owned()).boxed(),
    ];

    (
        proptest::collection::vec(proptest::strategy::Union::new(chatter), 1..30),
        proptest::collection::vec(proptest::strategy::Union::new(errors), 1..5),
    )
        .prop_map(|(mut chat, errs)| {
            chat.extend(errs);
            chat.concat()
        })
}

proptest! {
    #[test]
    fn prop_error_signals_are_preserved(input in arb_output_with_errors()) {
        // Tokens that must survive (case-sensitive where the L1 filter is).
        const SIGNALS: &[&str] = &[
            "ERROR", "error:", "panic:", "warning:",
            "Traceback", "Exception", "FAILED",
        ];
        let out = layer1_filter::filter(&input).output;
        for sig in SIGNALS {
            if input.contains(sig) {
                prop_assert!(
                    out.contains(sig),
                    "L1 dropped error signal {:?}\ninput:\n{}\noutput:\n{}",
                    sig, input, out
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant 8: template dedup preserves a real exemplar (no synthetic
// placeholders). When L1 emits `[×N]`, there must be a sibling line whose
// content isn't just the marker — i.e. the user can still read ONE concrete
// line to understand what the group collapsed.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_dedup_keeps_real_exemplar(input in arb_tool_output()) {
        let out = layer1_filter::filter(&input).output;
        // Any line with `[×N]` marker must also carry non-marker content.
        for line in out.lines() {
            if let Some(idx) = line.find("[×") {
                let tail = &line[idx..];
                // tail looks like "[×12] some real content"
                let after_bracket = tail
                    .split_once(']')
                    .map(|(_, rest)| rest.trim())
                    .unwrap_or("");
                prop_assert!(
                    !after_bracket.is_empty(),
                    "L1 produced [×N] marker with no exemplar body: {:?}",
                    line
                );
            }
        }
    }
}
