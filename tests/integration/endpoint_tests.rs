use axum_test::TestServer;
use ntk::compressor::layer3_backend::BackendKind;
use ntk::config::NtkConfig;
use ntk::metrics::MetricsStore;
use ntk::output::dashboard::WarnBuffer;
use ntk::server::{build_router, AppState};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn default_backend() -> Arc<BackendKind> {
    Arc::new(BackendKind::from_config(&NtkConfig::default()).unwrap())
}

fn empty_warn_log() -> WarnBuffer {
    Arc::new(Mutex::new(VecDeque::new()))
}

fn test_server() -> TestServer {
    test_server_with_token("")
}

/// Build a test server with an explicit shared-secret token. When empty,
/// the daemon's middleware falls back to the open-mode path (with a warn
/// log) — which mirrors the original test behaviour before auth landed.
fn test_server_with_token(token: &str) -> TestServer {
    let config = Arc::new(NtkConfig::default());
    let metrics = Arc::new(Mutex::new(MetricsStore::new()));
    let state = AppState {
        config,
        metrics,
        db: None,
        backend: default_backend(),
        started_at: std::time::Instant::now(),
        warn_log: empty_warn_log(),
        addr: "127.0.0.1:8765".to_string(),
        backend_name: "test".to_string(),
        model_info: String::new(),
        auth_token: Arc::new(token.to_string()),
    };
    let router = build_router(state);
    TestServer::new(router).expect("test server")
}

#[tokio::test]
async fn test_compress_endpoint_returns_compressed() {
    let server = test_server();

    // A large-enough input so Layer 1 can remove at least something.
    let output = "test foo::bar ... ok\n".repeat(50)
        + "test foo::baz ... FAILED\ntest result: FAILED. 49 passed; 1 failed";

    let resp = server
        .post("/compress")
        .json(&serde_json::json!({ "output": output, "command": "cargo test" }))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["layer"], 2);
    // Compressed output must be shorter than original.
    let compressed = body["compressed"].as_str().unwrap();
    assert!(compressed.len() < output.len(), "expected compression");
    // ratio must be > 0.
    assert!(body["ratio"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn test_compress_short_output_skips_layer3() {
    let server = test_server();

    let short = "nothing to commit, working tree clean";
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({ "output": short }))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    // Short input: tokens < threshold (300) → layer 2, not 3.
    assert!(body["tokens_after"].as_u64().unwrap() < 300);
    assert_eq!(body["layer"], 2);
}

#[tokio::test]
async fn test_health_endpoint() {
    let server = test_server();
    let resp = server.get("/health").await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "ok");
    assert!(body["version"].as_str().is_some());
    assert!(body["model"].as_str().is_some());
}

#[tokio::test]
async fn test_metrics_endpoint_after_compression() {
    let server = test_server();

    // Perform a compression first.
    server
        .post("/compress")
        .json(&serde_json::json!({ "output": "cargo:warning=foo\n".repeat(30) }))
        .await;

    let resp = server.get("/metrics").await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["total_compressions"], 1);
    assert!(body["total_tokens_saved"].as_u64().unwrap() >= 0);
}

#[tokio::test]
async fn test_compress_rejects_oversized_input() {
    // Create a config with a very small max_input_chars to trigger the limit.
    use ntk::config::{ExclusionsConfig, NtkConfig};
    let mut config = NtkConfig::default();
    config.exclusions = ExclusionsConfig {
        max_input_chars: 10,
        ..Default::default()
    };
    let state = AppState {
        config: Arc::new(config),
        metrics: Arc::new(Mutex::new(MetricsStore::new())),
        db: None,
        backend: default_backend(),
        started_at: std::time::Instant::now(),
        warn_log: empty_warn_log(),
        addr: "127.0.0.1:8765".to_string(),
        backend_name: "test".to_string(),
        model_info: String::new(),
        auth_token: Arc::new(String::new()),
    };
    let server = TestServer::new(build_router(state)).expect("test server");

    let resp = server
        .post("/compress")
        .json(&serde_json::json!({ "output": "this is longer than 10 chars" }))
        .await;

    resp.assert_status(axum::http::StatusCode::PAYLOAD_TOO_LARGE);
}

// --- Auth token (X-NTK-Token) ---------------------------------------------

#[tokio::test]
async fn test_compress_rejected_without_auth_token() {
    // Given a server configured with a real token, a request that does not
    // carry X-NTK-Token must get 401.
    let server = test_server_with_token("s3cret-token");
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({ "output": "some output longer than ten chars" }))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_compress_accepted_with_correct_auth_token() {
    let server = test_server_with_token("s3cret-token");
    let resp = server
        .post("/compress")
        .add_header(
            axum::http::HeaderName::from_static("x-ntk-token"),
            axum::http::HeaderValue::from_static("s3cret-token"),
        )
        .json(&serde_json::json!({ "output": "some output longer than ten chars" }))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn test_health_endpoint_open_without_token() {
    // Health stays open for liveness probes and `ntk status`.
    let server = test_server_with_token("s3cret-token");
    let resp = server.get("/health").await;
    resp.assert_status_ok();
}

// --- Layer 4 (context injection) — endpoint-level tests ------------------

#[tokio::test]
async fn test_compress_accepts_context_field() {
    // Invariant: /compress must accept the L4 `context` field without error,
    // even when L3 is disabled or the input is too small to trigger L3.
    let server = test_server();
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({
            "output": "cargo:warning=foo\n".repeat(30),
            "command": "cargo build",
            "context": "I am debugging a compilation error and need the actual error line."
        }))
        .await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    // L1/L2 must still run and produce a shorter output.
    assert!(body["compressed"].as_str().is_some());
    assert!(body["tokens_after"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_compress_accepts_transcript_path() {
    // Write a minimal Claude Code transcript with a user message.
    let tmp = std::env::temp_dir().join(format!("ntk_l4_test_{}.jsonl", std::process::id()));
    std::fs::write(
        &tmp,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"find the failing test"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}
"#,
    )
    .unwrap();

    let server = test_server();
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({
            "output": "cargo:warning=foo\n".repeat(30),
            "transcript_path": tmp.to_string_lossy(),
        }))
        .await;

    let status = resp.status_code();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(status.as_u16(), 200, "expected 200, got {status}");
}

#[tokio::test]
async fn test_compress_graceful_on_missing_transcript() {
    // Invariant (l4-context-injection.md #3): any L4 failure falls back silently.
    // Pointing transcript_path at a non-existent file must NOT return 500.
    let server = test_server();
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({
            "output": "cargo:warning=foo\n".repeat(30),
            "transcript_path": "/this/path/definitely/does/not/exist.jsonl"
        }))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn test_compress_accepts_cwd_field() {
    // cwd is forwarded by the hook for metric annotation; must not break anything.
    let server = test_server();
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({
            "output": "cargo:warning=foo\n".repeat(30),
            "cwd": "/tmp/some/project"
        }))
        .await;
    resp.assert_status_ok();
}
