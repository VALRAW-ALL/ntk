// ---------------------------------------------------------------------------
// Quality Regression Tests
//
// Each test asserts a minimum compression ratio AND that critical information
// (errors, failure counts, file paths) is preserved in the compressed output.
//
// Purpose: detect regressions where a change to Layer 1 or Layer 2 reduces
// compression quality or destroys important information.
//
// Run with: cargo test --test quality_regression_tests
// ---------------------------------------------------------------------------

use ntk::compressor::{layer1_filter, layer2_tokenizer};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn compress_fixture(name: &str) -> (String, usize, usize, f64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let input = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));

    let original_tokens = layer2_tokenizer::count_tokens(&input)
        .expect("token count failed");

    let l1 = layer1_filter::filter(&input);
    let l2 = layer2_tokenizer::process(&l1.output).expect("layer2 failed");

    let ratio = if original_tokens > 0 {
        let saved = original_tokens.saturating_sub(l2.compressed_tokens);
        saved as f64 / original_tokens as f64
    } else {
        0.0
    };

    (l2.output, original_tokens, l2.compressed_tokens, ratio)
}

fn assert_min_ratio(fixture: &str, min_ratio: f64) {
    let (output, before, after, ratio) = compress_fixture(fixture);
    assert!(
        ratio >= min_ratio,
        "Quality regression in {fixture}: ratio={:.1}% < required {:.1}%\n\
         before={before} tokens, after={after} tokens\n\
         Output preview: {}",
        ratio * 100.0,
        min_ratio * 100.0,
        &output[..output.len().min(200)],
    );
}

fn assert_content_preserved(fixture: &str, required_substrings: &[&str]) {
    let (output, _, _, _) = compress_fixture(fixture);
    for substr in required_substrings {
        assert!(
            output.contains(substr),
            "Quality regression: '{substr}' missing from compressed output of {fixture}\n\
             Output preview: {}",
            &output[..output.len().min(500)],
        );
    }
}

// ---------------------------------------------------------------------------
// cargo test output
// ---------------------------------------------------------------------------

#[test]
fn test_cargo_test_compression_ratio() {
    // cargo_test_output.txt has 47 passing tests + 1 failure.
    // L1 strips "... ok" lines → significant compression. Minimum 55%.
    assert_min_ratio("cargo_test_output.txt", 0.55);
}

#[test]
fn test_cargo_test_preserves_failure_info() {
    // The compressed output must still identify the failing test.
    assert_content_preserved("cargo_test_output.txt", &["FAILED"]);
}

// ---------------------------------------------------------------------------
// TypeScript compiler (tsc) output
// ---------------------------------------------------------------------------

#[test]
fn test_tsc_compression_ratio() {
    // The tsc fixture is small (28 lines) — every error line matters.
    // Minimum 3% prevents regressions where layer1 inflates output.
    assert_min_ratio("tsc_output.txt", 0.03);
}

#[test]
fn test_tsc_preserves_error_info() {
    // TS errors include file.ts(line,col): error TSxxxx
    assert_content_preserved("tsc_output.txt", &["error"]);
}

// ---------------------------------------------------------------------------
// Vitest output
// ---------------------------------------------------------------------------

#[test]
fn test_vitest_compression_ratio() {
    // Small fixture (27 lines). Minimum 2% regression guard.
    assert_min_ratio("vitest_output.txt", 0.02);
}

#[test]
fn test_vitest_preserves_test_result() {
    // Must contain some indication of test results.
    let (output, _, _, _) = compress_fixture("vitest_output.txt");
    let has_result = output.contains("pass")
        || output.contains("fail")
        || output.contains("PASS")
        || output.contains("FAIL")
        || output.contains("ok")
        || output.contains("✓")
        || output.contains("×");
    assert!(
        has_result,
        "Vitest compressed output should contain pass/fail indicators:\n{}",
        &output[..output.len().min(400)]
    );
}

// ---------------------------------------------------------------------------
// Docker logs
// ---------------------------------------------------------------------------

#[test]
fn test_docker_logs_non_empty_output() {
    let (output, _, _, _) = compress_fixture("docker_logs.txt");
    assert!(!output.is_empty(), "Compressed docker logs must not be empty");
}

// ---------------------------------------------------------------------------
// Next.js build
// ---------------------------------------------------------------------------

#[test]
fn test_next_build_non_empty_output() {
    let (output, _, _, _) = compress_fixture("next_build_output.txt");
    assert!(!output.is_empty(), "Compressed next build must not be empty");
}

// ---------------------------------------------------------------------------
// RTK pre-filtered output (already compressed by RTK)
// ---------------------------------------------------------------------------

#[test]
fn test_rtk_filtered_output_still_valid() {
    // RTK-filtered output is already lean — NTK must at least not break it.
    let (output, _, _, _) = compress_fixture("cargo_test_rtk_filtered.txt");
    assert!(!output.is_empty(), "RTK-pre-filtered output must survive NTK");
}

// ---------------------------------------------------------------------------
// Determinism: same input → same output every time
// ---------------------------------------------------------------------------

#[test]
fn test_compression_is_deterministic() {
    let fixtures = [
        "cargo_test_output.txt",
        "tsc_output.txt",
        "vitest_output.txt",
    ];
    for fixture in &fixtures {
        let (out1, _, _, _) = compress_fixture(fixture);
        let (out2, _, _, _) = compress_fixture(fixture);
        assert_eq!(
            out1, out2,
            "Compression is not deterministic for {fixture}"
        );
    }
}

// ---------------------------------------------------------------------------
// Token count never increases after compression
// ---------------------------------------------------------------------------

#[test]
fn test_token_count_never_increases() {
    let fixtures = [
        "cargo_test_output.txt",
        "tsc_output.txt",
        "vitest_output.txt",
        "docker_logs.txt",
        "next_build_output.txt",
    ];
    for fixture in &fixtures {
        let (_, before, after, _) = compress_fixture(fixture);
        assert!(
            after <= before,
            "Token count increased for {fixture}: {before} → {after}"
        );
    }
}

// ---------------------------------------------------------------------------
// Compression summary report (informational — always passes)
// ---------------------------------------------------------------------------

#[test]
fn test_print_quality_summary() {
    let fixtures = [
        ("cargo_test_output.txt", "cargo test"),
        ("tsc_output.txt", "tsc"),
        ("vitest_output.txt", "vitest"),
        ("docker_logs.txt", "docker logs"),
        ("next_build_output.txt", "next build"),
    ];

    println!("\n{:<20}  {:>8}  {:>8}  {:>8}", "FIXTURE", "BEFORE", "AFTER", "RATIO");
    println!("{}", "-".repeat(52));

    for (file, label) in &fixtures {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(file);
        if !path.exists() {
            continue;
        }
        let (_, before, after, ratio) = compress_fixture(file);
        println!(
            "{:<20}  {:>8}  {:>8}  {:>7.1}%",
            label, before, after,
            ratio * 100.0
        );
    }
    println!();
    // This test always passes — it just prints the summary.
}
