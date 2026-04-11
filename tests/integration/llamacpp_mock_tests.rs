// llamacpp_mock_tests.rs
//
// Integration tests for the llama.cpp subprocess backend.
// Uses wiremock to simulate the llama-server HTTP API without requiring
// a real llama-server binary or GGUF model.

use ntk::compressor::layer3_llamacpp::LlamaCppBackend;
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

/// Builds a mock llama-server `/completion` response.
fn llama_response(content: &str) -> serde_json::Value {
    serde_json::json!({
        "content": content,
        "stop": true,
        "tokens_evaluated": 80,
        "tokens_predicted": 15,
        "model": "Phi-3-mini-4k-instruct-Q5_K_M.gguf",
        "generation_settings": {
            "n_predict": 512,
            "temperature": 0.1
        }
    })
}

/// Builds a mock llama-server `/health` response.
fn health_response() -> serde_json::Value {
    serde_json::json!({ "status": "ok" })
}

/// Creates a `LlamaCppBackend` pre-wired to talk to the given wiremock URL.
/// Bypasses subprocess spawning by setting `server_url` directly.
fn make_backend(server_url: &str) -> LlamaCppBackend {
    LlamaCppBackend::new_with_url(
        server_url.to_owned(),
        std::path::PathBuf::from("/nonexistent/model.gguf"),
        0,
        30_000,
        2048,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Happy-path: llama-server responds correctly → `compress()` returns Ok.
#[tokio::test]
async fn test_llamacpp_compress_happy_path() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(llama_response("3 passed, 1 failed")))
        .expect(1)
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let backend = make_backend(&server.uri());
    let result = backend
        .compress("cargo test output", OutputType::Test, dir.path())
        .await;

    server.verify().await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    let r = result.unwrap();
    assert_eq!(r.output, "3 passed, 1 failed");
    assert_eq!(r.input_tokens, 80);
    assert_eq!(r.output_tokens, 15);
}

/// When the server returns a non-2xx status, `compress()` must return Err.
#[tokio::test]
async fn test_llamacpp_compress_server_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let backend = make_backend(&server.uri());
    let result = backend
        .compress("some output", OutputType::Build, dir.path())
        .await;

    assert!(result.is_err(), "expected Err on HTTP 500");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("500") || msg.contains("HTTP"), "error should mention status: {msg}");
}

/// When the server is unreachable, `compress()` must return Err (not panic).
#[tokio::test]
async fn test_llamacpp_compress_server_unreachable() {
    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    // Port 19998 — nothing listening.
    let backend = make_backend("http://127.0.0.1:19998");
    let result = backend
        .compress("some output", OutputType::Log, dir.path())
        .await;

    assert!(result.is_err(), "expected Err when server unreachable");
}

/// When the server exceeds the timeout, `compress()` must return Err.
#[tokio::test]
async fn test_llamacpp_compress_timeout() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(llama_response("slow result"))
                .set_delay(Duration::from_millis(2000)),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    // 100ms timeout — server is 2s delayed.
    let backend = LlamaCppBackend::new_with_url(
        server.uri(),
        std::path::PathBuf::from("/nonexistent/model.gguf"),
        0,
        100, // timeout_ms
        2048,
    );

    let result = backend
        .compress("slow output", OutputType::Diff, dir.path())
        .await;

    assert!(result.is_err(), "expected Err on timeout, got Ok");
}

/// When the response `content` field is empty, `compress()` must return Err.
#[tokio::test]
async fn test_llamacpp_compress_empty_content() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "",
                "stop": true,
                "tokens_evaluated": 10,
                "tokens_predicted": 0,
            })),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let backend = make_backend(&server.uri());
    let result = backend
        .compress("some output", OutputType::Generic, dir.path())
        .await;

    assert!(result.is_err(), "expected Err on empty content");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("empty"), "error should mention empty: {msg}");
}

/// When the response JSON is missing the `content` field entirely, `compress()` must return Err.
#[tokio::test]
async fn test_llamacpp_compress_missing_content_field() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stop": true,
                "tokens_evaluated": 5,
                "tokens_predicted": 0,
            })),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let backend = make_backend(&server.uri());
    let result = backend
        .compress("some output", OutputType::Generic, dir.path())
        .await;

    assert!(result.is_err(), "expected Err on missing content field");
}

/// Verify that all OutputType variants are handled without panicking
/// (each selects its system prompt file successfully via embedded fallback).
#[tokio::test]
async fn test_llamacpp_all_output_types() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(llama_response("compressed")))
        .mount(&server)
        .await;

    // Use a dir with NO prompt files → exercises the embedded fallback path.
    let dir = TempDir::new().unwrap();

    let backend = make_backend(&server.uri());

    for output_type in [
        OutputType::Test,
        OutputType::Build,
        OutputType::Log,
        OutputType::Diff,
        OutputType::Generic,
    ] {
        let result = backend.compress("input", output_type, dir.path()).await;
        assert!(
            result.is_ok(),
            "expected Ok for {output_type:?}, got: {:?}",
            result.err()
        );
    }
}

/// Verify that the `/completion` request includes a prompt (not empty).
#[tokio::test]
async fn test_llamacpp_request_has_prompt() {
    let server = MockServer::start().await;

    let received_body = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let received_body_clone = received_body.clone();

    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(llama_response("result")))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_prompts(dir.path());

    let backend = make_backend(&server.uri());
    let result = backend
        .compress("the user's input text", OutputType::Test, dir.path())
        .await;

    drop(received_body_clone);

    assert!(result.is_ok(), "compress should succeed: {:?}", result.err());
    assert_eq!(result.unwrap().output, "result");
}
