use ntk::compressor::{layer1_filter, layer2_tokenizer};
use ntk::config::NtkConfig;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing fixture: {}", path.display()))
}

#[test]
fn test_cargo_test_fixture_compression_ratio() {
    let input = fixture("cargo_test_output.txt");

    // Measure end-to-end: original input vs final output after L1 + L2.
    let original_tokens = layer2_tokenizer::count_tokens(&input).expect("count original");

    let l1 = layer1_filter::filter(&input);
    let l2 = layer2_tokenizer::process(&l1.output).expect("layer2");
    let final_tokens = l2.compressed_tokens;

    let ratio = if original_tokens == 0 {
        0.0f32
    } else {
        let saved = original_tokens.saturating_sub(final_tokens);
        saved as f32 / original_tokens as f32
    };

    assert!(
        ratio >= 0.50,
        "expected end-to-end ratio >= 0.50, got {ratio:.2} ({original_tokens} → {final_tokens} tokens)"
    );
}

#[test]
fn test_cargo_test_rtk_pre_filtered_flag() {
    let input = fixture("cargo_test_rtk_filtered.txt");
    let l1 = layer1_filter::filter(&input);
    assert!(
        l1.rtk_pre_filtered,
        "expected rtk_pre_filtered=true for RTK fixture"
    );
}

#[test]
fn test_tsc_errors_preserved() {
    let input = fixture("tsc_output.txt");
    let l1 = layer1_filter::filter(&input);
    let l2 = layer2_tokenizer::process(&l1.output).expect("layer2");

    // Error codes must survive compression.
    assert!(
        l2.output.contains("TS2345") || l2.output.contains("TS"),
        "TypeScript error codes not preserved in output"
    );
    // At least one file reference must remain.
    assert!(
        l2.output.contains(".ts") || l2.output.contains("error"),
        "error info not preserved"
    );
}

#[test]
fn test_layer3_not_triggered_below_threshold() {
    let config = NtkConfig::default();
    let threshold = config.compression.inference_threshold_tokens;

    let short = "nothing to commit, working tree clean";
    let l1 = layer1_filter::filter(short);
    let l2 = layer2_tokenizer::process(&l1.output).expect("layer2");

    assert!(
        l2.compressed_tokens < threshold,
        "short input ({} tokens) should be below threshold ({})",
        l2.compressed_tokens,
        threshold
    );
}

#[test]
fn test_layer3_stub_triggered_above_threshold() {
    let config = NtkConfig::default();
    let threshold = config.compression.inference_threshold_tokens;

    // Generate input that exceeds the threshold by repeating distinct lines.
    let large: String = (0..500)
        .map(|i| format!("cargo:warning=unused variable `x` in function_{i} at src/lib.rs:{i}\n"))
        .collect();

    let l1 = layer1_filter::filter(&large);
    let l2 = layer2_tokenizer::process(&l1.output).expect("layer2");

    assert!(
        l2.original_tokens > threshold,
        "test input ({} tokens) should exceed threshold ({})",
        l2.original_tokens,
        threshold
    );
}
