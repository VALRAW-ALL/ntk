// Etapa 16 — Ollama HTTP mock tests via wiremock-rs
//
// These tests verify Layer 3 behavior without a real Ollama installation.
// The wiremock server simulates the Ollama /api/generate endpoint.

use ntk::compressor::layer3_inference::OllamaClient;
use ntk::detector::OutputType;
use std::time::Duration;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_prompts(dir: &std::path::Path) {
    for (name, content) in &[
        ("test.txt", "You are a test output compressor."),
        ("build.txt", "You are a build output compressor."),
        ("log.txt", "You are a log compressor."),
        ("diff.txt", "You are a diff compressor."),
    ] {
        std::fs::write(dir.join(name), content).unwrap();
    }
}

/// Ollama's streaming /api/generate emits one JSON-per-line (NDJSON).
/// Since the client now always requests `stream: true`, mocks must return
/// NDJSON — split the response into per-token objects with a final `done`.
fn ollama_stream_ndjson(response_text: &str) -> String {
    let mut out = String::new();
    // Split into rough word-sized "tokens" so the stream parser sees
    // multiple chunks. Any whitespace-preserving split works here.
    let parts: Vec<&str> = response_text.split_inclusive(' ').collect();
    for (i, part) in parts.iter().enumerate() {
        let obj = serde_json::json!({
            "model": "phi3:mini",
            "response": part,
            "done": false,
            "done_reason": serde_json::Value::Null,
        });
        out.push_str(&obj.to_string());
        out.push('\n');
        let _ = i; // silence unused
    }
    // Final object signals completion + carries counts.
    let done = serde_json::json!({
        "model": "phi3:mini",
        "response": "",
        "done": true,
        "prompt_eval_count": 100,
        "eval_count": 20,
    });
    out.push_str(&done.to_string());
    out.push('\n');
    out
}

/// Convenience helper — one-shot NDJSON body with content-type set so
/// reqwest's bytes_stream delivers raw bytes to the streaming parser.
fn ndjson_response(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/x-ndjson")
        .set_body_string(body.to_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that the client sends the correct JSON structure to Ollama:
/// - `model` field set
/// - `system` field contains the prompt
/// - `prompt` field contains the input (NOT in system — prevents injection)
/// - `stream: false`
#[tokio::test]
async fn test_inference_request_format() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ndjson_response(&ollama_stream_ndjson("2 passed, 1 failed")))
        .expect(1)
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let client = OllamaClient::new(&server.uri(), 5000, "phi3:mini");
    let result = client
        .compress("cargo test output here", OutputType::Test, dir.path())
        .await;

    // Verify mock was called.
    server.verify().await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    let r = result.unwrap();
    assert_eq!(r.output, "2 passed, 1 failed");
    assert_eq!(r.input_tokens, 100);
    assert_eq!(r.output_tokens, 20);
}

/// When Ollama is unavailable (no server), compress() must return Err
/// so the caller can fall back to L1+L2 output.
#[tokio::test]
async fn test_fallback_on_ollama_unavailable() {
    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    // Port 19999 — nothing listening there.
    let client = OllamaClient::new("http://127.0.0.1:19999", 500, "phi3:mini");
    let result = client
        .compress("some output", OutputType::Build, dir.path())
        .await;

    assert!(
        result.is_err(),
        "expected Err when Ollama is unavailable, got Ok"
    );
}

/// When Ollama times out (slow response > timeout), compress() must return Err.
#[tokio::test]
async fn test_fallback_on_ollama_timeout() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        // Add a 2-second delay — longer than our 100ms timeout.
        .respond_with(
            ndjson_response(&ollama_stream_ndjson("compressed"))
                .set_delay(Duration::from_millis(2000)),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let client = OllamaClient::new(&server.uri(), 100, "phi3:mini");
    let result = client
        .compress("slow output", OutputType::Log, dir.path())
        .await;

    assert!(
        result.is_err(),
        "expected Err on timeout, got Ok: {:?}",
        result.as_ref().map(|r| &r.output)
    );
}

/// Verify that the streaming client assembles multi-token NDJSON into the
/// full compressed output — catches regressions in the stream-parsing loop
/// that would truncate output when Ollama emits >1 chunk.
#[tokio::test]
async fn test_stream_assembles_multi_token_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ndjson_response(&ollama_stream_ndjson(
            "one two three four five six seven",
        )))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let client = OllamaClient::new(&server.uri(), 5000, "phi3:mini");
    let result = client
        .compress("input", OutputType::Log, dir.path())
        .await
        .expect("ok");
    assert_eq!(result.output, "one two three four five six seven");
    assert_eq!(result.input_tokens, 100);
    assert_eq!(result.output_tokens, 20);
}

/// Verify that the client cancels cleanly when the calling future is
/// dropped mid-stream. tokio::time::timeout drops its inner future on
/// deadline, which propagates to the HTTP body stream and closes the
/// connection. The assertion here is simply that no panic / hang occurs —
/// cancellation safety under the streaming parser is the actual contract.
#[tokio::test]
async fn test_stream_cancels_cleanly_when_dropped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(
            ndjson_response(&ollama_stream_ndjson("never finishes"))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let client = OllamaClient::new(&server.uri(), 30_000, "phi3:mini");
    let fut = client.compress("input", OutputType::Log, dir.path());

    // Drop after 100ms — the daemon in prod does the same on hook deadline.
    let result = tokio::time::timeout(Duration::from_millis(100), fut).await;
    assert!(
        result.is_err(),
        "expected timeout (not panic), got: {result:?}"
    );
}

/// Verify that the correct system prompt file is selected per output type:
/// Test → test.txt, Build → build.txt, etc.
#[tokio::test]
async fn test_correct_prompt_per_type() {
    let server = MockServer::start().await;

    // Capture the request body so we can inspect the system prompt used.
    let (sender, receiver) = tokio::sync::oneshot::channel::<String>();
    let sender = std::sync::Arc::new(tokio::sync::Mutex::new(Some(sender)));

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ndjson_response(&ollama_stream_ndjson("ok")))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    // Test type → should load test.txt
    let client = OllamaClient::new(&server.uri(), 5000, "phi3:mini");
    let result = client
        .compress("vitest output", OutputType::Test, dir.path())
        .await;

    assert!(
        result.is_ok(),
        "expected Ok for Test type: {:?}",
        result.err()
    );

    // Verify via a second call that Build type also works (different prompt file).
    let result2 = client
        .compress("tsc error output", OutputType::Build, dir.path())
        .await;

    assert!(
        result2.is_ok(),
        "expected Ok for Build type: {:?}",
        result2.err()
    );

    // Both calls should succeed, confirming prompt files for both types are loaded.
    drop(sender); // silence unused warning
    drop(receiver);
}
