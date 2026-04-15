use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::OnceLock;
use tiktoken_rs::{cl100k_base, CoreBPE};

// ---------------------------------------------------------------------------
// Tokenizer singleton — initialized once, reused for every call.
// cl100k_base() parses the BPE vocab file (~50ms); caching it eliminates
// that cost on every compression request.
// ---------------------------------------------------------------------------

fn tokenizer() -> Result<&'static CoreBPE> {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    if let Some(bpe) = BPE.get() {
        return Ok(bpe);
    }
    let bpe = cl100k_base().context("failed to load cl100k_base tokenizer")?;
    Ok(BPE.get_or_init(|| bpe))
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct Layer2Result {
    pub output: String,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn process(input: &str) -> Result<Layer2Result> {
    let bpe = tokenizer()?;

    let original_tokens = bpe.encode_ordinary(input).len();

    let after_normalize = normalize_opaque_tokens(input);
    let after_paths = shorten_paths(&after_normalize);
    let after_whitespace = collapse_whitespace(&after_paths);
    let output = consolidate_prefixes(&after_whitespace);

    let compressed_tokens = bpe.encode_ordinary(&output).len();

    Ok(Layer2Result {
        output,
        original_tokens,
        compressed_tokens,
    })
}

// ---------------------------------------------------------------------------
// Normalize opaque tokens — URLs with query strings, long hashes, base64
// blobs, bearer tokens. These are token-expensive under BPE but carry no
// debugging signal beyond "there was an ID here".
//
// Replacements preserve a short prefix + "…" so matching across lines of a
// single trace still works visually.
// ---------------------------------------------------------------------------

// Static regex literals — `.expect(..)` here can only fail if the literal
// pattern itself is malformed (tested in CI), so suppressing the lint is
// safe while keeping the security-gate clippy rules on elsewhere.
#[allow(clippy::expect_used)]
static RE_URL_QUERY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\?[A-Za-z0-9=&_%+.-]{20,}").expect("url query regex must compile"));

#[allow(clippy::expect_used)]
static RE_JWT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}")
        .expect("jwt regex must compile")
});

#[allow(clippy::expect_used)]
static RE_LONG_HASH: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{32,}\b").expect("long-hash regex must compile"));

#[allow(clippy::expect_used)]
static RE_BASE64_BLOB: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Za-z0-9+/]{40,}={0,2}").expect("base64 regex must compile"));

fn normalize_opaque_tokens(input: &str) -> String {
    // Order matters: longer / more-specific patterns first so they consume
    // bytes before the generic base64 regex sees them.
    let s1 = RE_JWT.replace_all(input, "<JWT>");
    let s2 = RE_LONG_HASH.replace_all(&s1, |caps: &regex::Captures| {
        // Preserve a short prefix for visual traceability: sha256 "abc12345…"
        let full = &caps[0];
        if full.len() >= 12 {
            format!("{}...", &full[..8])
        } else {
            "<HASH>".to_owned()
        }
    });
    let s3 = RE_URL_QUERY.replace_all(&s2, "?<QUERY>");
    let s4 = RE_BASE64_BLOB.replace_all(&s3, "<B64>");
    s4.into_owned()
}

// ---------------------------------------------------------------------------
// Smart whitespace collapse
//
// BPE charges extra for long runs of spaces ("          " tokenizes as
// several tokens). Collapsing these to a single space (while preserving
// meaningful indentation up to 4 columns) is safe and cheap.
//
// Rules:
//   - Leading whitespace up to 4 columns is preserved verbatim.
//   - Runs of ≥ 3 spaces in the middle of a line collapse to 2 spaces.
//   - Trailing whitespace is stripped.
//   - Tabs are preserved (they often convey semantic indentation).
// ---------------------------------------------------------------------------

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for (i, line) in input.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }

        // Preserve up to 4 cols of leading space.
        let leading: String = line.chars().take_while(|c| *c == ' ').take(4).collect();
        out.push_str(&leading);

        let mut rest = &line[leading.len()..];
        // Skip the leading spaces we already emitted.
        rest = rest.trim_start_matches(' ');
        // Collapse mid-line runs of ≥ 3 spaces to 2.
        let mut space_run = 0usize;
        for c in rest.chars() {
            if c == ' ' {
                space_run = space_run.saturating_add(1);
                if space_run <= 2 {
                    out.push(' ');
                }
            } else {
                space_run = 0;
                out.push(c);
            }
        }

        // Strip trailing whitespace by truncating.
        while out.ends_with(' ') || out.ends_with('\t') {
            out.pop();
        }
    }
    out
}

/// Count tokens in a string using cl100k_base.
pub fn count_tokens(input: &str) -> Result<usize> {
    Ok(tokenizer()?.encode_ordinary(input).len())
}

// ---------------------------------------------------------------------------
// Step 1 — Shorten absolute/long paths
//
// "src/components/Button/index.tsx:10:5" → "Button/index.tsx:10"
// "error at /home/user/project/src/foo/bar/baz.rs:42" → "baz.rs:42"
//
// Rules:
// - Keep only the last 2 path segments (dirname + filename) plus line:col.
// - Only shorten paths with 3+ segments to avoid altering short paths.
// ---------------------------------------------------------------------------

fn shorten_paths(input: &str) -> String {
    // Regex-free implementation: scan word by word looking for path-like tokens.
    // A path token: contains '/' or '\', ends optionally with ':\d+'
    let mut result = String::with_capacity(input.len());

    for line in input.lines() {
        let shortened = shorten_line_paths(line);
        result.push_str(&shortened);
        result.push('\n');
    }

    // Remove the trailing newline we always add.
    if result.ends_with('\n') && !input.ends_with('\n') {
        result.pop();
    }

    result
}

fn shorten_line_paths(line: &str) -> String {
    // Split by whitespace boundaries, process each token.
    // We use a simple char-by-char scan to avoid regex dependency.
    let mut out = String::with_capacity(line.len());
    let mut token_start = 0usize;

    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0usize;

    while i <= len {
        let is_boundary = i == len
            || chars[i].is_whitespace()
            || chars[i] == ','
            || chars[i] == ')'
            || chars[i] == '('
            || chars[i] == '\''
            || chars[i] == '"';

        if is_boundary {
            let token: String = chars[token_start..i].iter().collect();
            out.push_str(&maybe_shorten_path(&token));

            if i < len {
                out.push(chars[i]);
            }
            token_start = i.saturating_add(1);
        }

        i = i.saturating_add(1);
    }

    out
}

/// If `token` looks like a multi-segment path, return the last 2 segments.
/// Preserves trailing `:line` or `:line:col` suffixes.
fn maybe_shorten_path(token: &str) -> String {
    if token.is_empty() {
        return token.to_owned();
    }

    // Split off trailing :digits:digits or :digits suffix.
    let (path_part, suffix) = split_path_suffix(token);

    // Count path separators.
    let sep_count = path_part
        .chars()
        .filter(|c| matches!(c, '/' | '\\'))
        .count();
    if sep_count < 2 {
        // Not deep enough to shorten.
        return token.to_owned();
    }

    // Find the last 2 segments.
    let segments: Vec<&str> = path_part
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect();

    if segments.len() < 2 {
        return token.to_owned();
    }

    let short = format!(
        "{}/{}{}",
        segments[segments.len().saturating_sub(2)],
        segments[segments.len().saturating_sub(1)],
        suffix
    );

    short
}

/// Split "src/foo/bar.rs:10:5" into ("src/foo/bar.rs", ":10:5").
fn split_path_suffix(token: &str) -> (&str, &str) {
    // Walk backwards: if the token ends with :\d+ (optionally repeated), strip it.
    let bytes = token.as_bytes();
    let mut end = bytes.len();

    loop {
        if end == 0 {
            break;
        }
        // Look for ':' followed by only digits up to `end`.
        if let Some(colon_pos) = token[..end].rfind(':') {
            let after = &token[colon_pos.saturating_add(1)..end];
            if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
                end = colon_pos;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    (&token[..end], &token[end..])
}

// ---------------------------------------------------------------------------
// Step 2 — Consolidate repeated prefixes
//
// Before:
//   ERROR: src/a.ts
//   ERROR: src/b.ts
//   ERROR: src/c.ts
//
// After:
//   ERROR: a.ts, b.ts, c.ts
//
// Rule: consecutive lines that share an identical prefix up to the last
// space+token are grouped. Prefix must be ≥ 4 chars.
// ---------------------------------------------------------------------------

fn consolidate_prefixes(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut idx = 0usize;

    while idx < lines.len() {
        let current = lines[idx];
        let prefix = line_prefix(current);

        if prefix.len() < 4 {
            out.push(current.to_owned());
            idx = idx.saturating_add(1);
            continue;
        }

        // Collect all consecutive lines with the same prefix.
        let mut suffixes: Vec<&str> = vec![line_suffix(current, &prefix)];
        let mut lookahead = idx.saturating_add(1);

        while lookahead < lines.len() {
            let next_prefix = line_prefix(lines[lookahead]);
            if next_prefix == prefix {
                suffixes.push(line_suffix(lines[lookahead], &prefix));
                lookahead = lookahead.saturating_add(1);
            } else {
                break;
            }
        }

        if lookahead.saturating_sub(idx) > 1 {
            // Multiple lines share the prefix — consolidate.
            let joined = suffixes.join(", ");
            out.push(format!("{prefix}{joined}"));
        } else {
            out.push(current.to_owned());
        }

        idx = lookahead;
    }

    out.join("\n")
}

/// Extract the "prefix" of a line: everything up to and including the last
/// separator (`: `, `= `, `\t`).
fn line_prefix(line: &str) -> String {
    // Try ": " separator first (most common: "ERROR: file.ts").
    if let Some(pos) = line.find(": ") {
        return line[..pos.saturating_add(2)].to_owned();
    }
    // Try "= " separator.
    if let Some(pos) = line.find("= ") {
        return line[..pos.saturating_add(2)].to_owned();
    }
    // Try tab separator.
    if let Some(pos) = line.find('\t') {
        return line[..pos.saturating_add(1)].to_owned();
    }
    String::new()
}

fn line_suffix<'a>(line: &'a str, prefix: &str) -> &'a str {
    line.strip_prefix(prefix).unwrap_or(line)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_count_accuracy() {
        // "hello world" is 2 tokens in cl100k_base.
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
        // Shortened path should still contain filename and line number.
        assert!(result.output.contains("index.tsx"));
        assert!(result.output.contains("42"));
    }

    #[test]
    fn test_prefix_consolidation() {
        let input = "ERROR: src/a.ts\nERROR: src/b.ts\nERROR: src/c.ts";
        let result = process(input).unwrap();
        // All three should collapse to a single line.
        let lines: Vec<&str> = result.output.lines().collect();
        assert_eq!(lines.len(), 1, "expected 1 line, got: {:?}", lines);
        assert!(result.output.contains("a.ts"));
        assert!(result.output.contains("b.ts"));
        assert!(result.output.contains("c.ts"));
    }

    #[test]
    fn test_no_data_loss_in_reformatting() {
        // All distinct tokens in the input must appear in the output.
        let input = "error at module::foo: type mismatch\nnote: expected i32, found str";
        let result = process(input).unwrap();
        assert!(result.output.contains("type mismatch"));
        assert!(result.output.contains("expected i32"));
        assert!(result.output.contains("found str"));
    }

    #[test]
    fn test_threshold_not_triggered_below_300() {
        // Layer 3 threshold logic lives in the pipeline, but we verify here
        // that a short input counts fewer than 300 tokens after Layer 2.
        let short_input = "git status: nothing to commit, working tree clean";
        let result = process(short_input).unwrap();
        assert!(
            result.compressed_tokens < 300,
            "expected < 300 tokens, got {}",
            result.compressed_tokens
        );
    }

    // ----- New tests ---------------------------------------------------

    #[test]
    fn test_whitespace_collapse_mid_line() {
        let input = "key     value     more_value";
        let result = process(input).unwrap();
        assert_eq!(result.output.trim_end(), "key  value  more_value");
    }

    #[test]
    fn test_whitespace_preserve_leading_indent() {
        let input = "    indented line";
        let result = process(input).unwrap();
        assert!(result.output.starts_with("    indented"));
    }

    #[test]
    fn test_whitespace_strip_trailing() {
        let input = "hello world     ";
        let result = process(input).unwrap();
        assert!(!result.output.ends_with(' '));
    }

    #[test]
    fn test_long_hash_normalized() {
        let input = "commit abc1234567890abcdef1234567890abcdef12345678 is bad";
        let result = process(input).unwrap();
        assert!(
            result.compressed_tokens < result.original_tokens,
            "hash should reduce tokens"
        );
        assert!(result.output.contains("..."));
    }

    #[test]
    fn test_jwt_normalized() {
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let result = process(input).unwrap();
        assert!(result.output.contains("<JWT>"));
    }

    #[test]
    fn test_url_query_normalized() {
        let input = "GET /api/items?session=abcd1234efgh5678ijkl&sort=asc";
        let result = process(input).unwrap();
        assert!(result.output.contains("<QUERY>"));
    }

    #[test]
    fn test_no_loss_of_short_hex() {
        // Short hex (git SHAs ≤ 12 chars) should not be normalized
        // because they're still human-readable and useful in error messages.
        let input = "commit abc1234 is good";
        let result = process(input).unwrap();
        assert!(
            result.output.contains("abc1234"),
            "short hex should survive: {}",
            result.output
        );
    }
}
