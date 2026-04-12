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
    };
    let server = TestServer::new(build_router(state)).expect("test server");

    let resp = server
        .post("/compress")
        .json(&serde_json::json!({ "output": "this is longer than 10 chars" }))
        .await;

    resp.assert_status(axum::http::StatusCode::PAYLOAD_TOO_LARGE);
}
