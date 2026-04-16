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

// --- Opaque token normalization (v0.2.27+) ------------------------------

#[test]
fn test_opaque_jwt_is_shortened() {
    let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJ1c2VyX2lkIjoxMjMsImV4cCI6MTczMjAwMDAwMH0.k3pQY8R3zN2XhF4mA1vD2wKJ9sP5bG7LhT0oE8uY2cQ";
    let result = process(input).unwrap();
    assert!(
        result.compressed_tokens < result.original_tokens,
        "JWT should be shortened: {} → {}",
        result.original_tokens,
        result.compressed_tokens
    );
    // Scheme + field name must still be readable.
    assert!(result.output.contains("Authorization"));
    assert!(result.output.contains("Bearer"));
}

#[test]
fn test_opaque_long_hash_is_shortened() {
    let input = "commit 7f8e2d3c4b5a6f7e8d9c0b1a2e3f4d5c6b7a8e9f0d1c2b3a4e5f6d7c8b9a0 added";
    let result = process(input).unwrap();
    // 64-char hex must be truncated; "added" must survive.
    assert!(result.output.contains("added"));
    assert!(result.compressed_tokens < result.original_tokens);
}

#[test]
fn test_opaque_url_query_is_shortened() {
    let input = "GET /api?token=abcdef1234567890abcdef1234567890&next=/home 200";
    let result = process(input).unwrap();
    // Status code must survive and the query blob must be collapsed.
    assert!(result.output.contains("200"));
    assert!(
        !result
            .output
            .contains("token=abcdef1234567890abcdef1234567890"),
        "raw query string must not survive: {}",
        result.output
    );
    assert!(
        result.compressed_tokens < result.original_tokens,
        "tokens should decrease: {} → {}",
        result.original_tokens,
        result.compressed_tokens
    );
}

// --- Whitespace collapse -------------------------------------------------

#[test]
fn test_whitespace_collapse_preserves_leading_indent() {
    // Stack-frame indent (4 cols) must survive so structure stays readable.
    let input = "error:\n    at foo(bar.js:10)\n    at baz(bar.js:12)";
    let result = process(input).unwrap();
    assert!(
        result.output.contains("    at foo"),
        "4-col leading indent must survive: {:?}",
        result.output
    );
}

#[test]
fn test_whitespace_collapse_trims_trailing() {
    let input = "line1   \nline2\t\t\n";
    let result = process(input).unwrap();
    // No line should end with trailing whitespace.
    for line in result.output.lines() {
        assert_eq!(
            line,
            line.trim_end(),
            "line has trailing whitespace: {:?}",
            line
        );
    }
}
