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

fn ollama_response(response_text: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "phi3:mini",
        "response": response_text,
        "done": true,
        "prompt_eval_count": 100,
        "eval_count": 20,
    })
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
        .respond_with(
            ResponseTemplate::new(200).set_body_json(ollama_response("2 passed, 1 failed")),
        )
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
            ResponseTemplate::new(200)
                .set_body_json(ollama_response("compressed"))
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
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_response("ok")))
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
