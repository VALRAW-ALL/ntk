use ntk::compressor::layer2_tokenizer::process;

#[test]
fn test_token_count_accuracy() {
    let result = process("hello world").unwrap();
    assert_eq!(result.original_tokens, 2);
}

#[test]
fn test_path_shortening_reduces_tokens() {
    let long_path = "src/components/auth/LoginForm/index.tsx:42:10: error TS2345";
    let result = process(long_path).unwrap();
    assert!(
        result.compressed_tokens < result.original_tokens,
        "expected {} < {}",
        result.compressed_tokens,
        result.original_tokens
    );
    assert!(result.output.contains("index.tsx"));
    assert!(result.output.contains("42"));
}

#[test]
fn test_prefix_consolidation() {
    let input = "ERROR: src/a.ts\nERROR: src/b.ts\nERROR: src/c.ts";
    let result = process(input).unwrap();
    let lines: Vec<&str> = result.output.lines().collect();
    assert_eq!(lines.len(), 1, "expected 1 line, got: {:?}", lines);
    assert!(result.output.contains("a.ts"));
    assert!(result.output.contains("b.ts"));
    assert!(result.output.contains("c.ts"));
}

#[test]
fn test_no_data_loss_in_reformatting() {
    let input = "error at module::foo: type mismatch\nnote: expected i32, found str";
    let result = process(input).unwrap();
    assert!(result.output.contains("type mismatch"));
    assert!(result.output.contains("expected i32"));
    assert!(result.output.contains("found str"));
}

#[test]
fn test_threshold_not_triggered_below_300() {
    let short_input = "git status: nothing to commit, working tree clean";
    let result = process(short_input).unwrap();
    assert!(
        result.compressed_tokens < 300,
        "expected < 300 tokens, got {}",
        result.compressed_tokens
    );
}
