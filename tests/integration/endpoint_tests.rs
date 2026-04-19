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

// --- Audit log ------------------------------------------------------------

#[tokio::test]
async fn test_audit_log_appends_record_when_enabled() {
    use ntk::config::{NtkConfig, SecurityConfig};
    let tmp = tempfile::tempdir().expect("tempdir");
    let audit_path = tmp.path().join("audit.log");

    let mut config = NtkConfig::default();
    config.security = SecurityConfig {
        audit_log: true,
        audit_log_path: audit_path.to_string_lossy().into_owned(),
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

    let output = "cargo:warning=noise\n".repeat(40);
    let resp = server
        .post("/compress")
        .json(&serde_json::json!({ "output": output, "command": "cargo build" }))
        .await;
    resp.assert_status_ok();

    let contents = std::fs::read_to_string(&audit_path).expect("audit log file");
    assert!(
        contents.trim().lines().count() >= 1,
        "expected at least one audit record, got:\n{contents}"
    );
    let line = contents.lines().next().expect("at least one line");
    let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
    assert_eq!(parsed["command"], "cargo build");
    assert!(parsed["output_sha256"].as_str().expect("hash").len() == 64);
    // The raw output must never appear in the audit log.
    assert!(
        !contents.contains("cargo:warning=noise"),
        "audit log leaked raw output: {contents}"
    );
}

// --- L3 cache (SQLite-backed) ---------------------------------------------

#[tokio::test]
async fn test_l3_cache_roundtrip_and_ttl() {
    use ntk::metrics::MetricsDb;
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("metrics.db");
    let db = MetricsDb::init(&db_path).await.expect("init db");

    let key = MetricsDb::l3_cache_key("l2_out", "ctx", "ollama", "prefix");

    // Miss on empty table.
    assert!(db.lookup_l3_cache(&key, 7).await.expect("lookup").is_none());

    // Store then hit.
    db.store_l3_cache(&key, "summary", "ollama")
        .await
        .expect("store");
    let hit = db.lookup_l3_cache(&key, 7).await.expect("lookup2");
    assert_eq!(hit.as_deref(), Some("summary"));
    assert_eq!(db.l3_cache_size().await.expect("size"), 1);

    // Re-store with a new output overwrites the row (INSERT OR REPLACE).
    db.store_l3_cache(&key, "updated", "ollama")
        .await
        .expect("restore");
    assert_eq!(
        db.lookup_l3_cache(&key, 7)
            .await
            .expect("lookup3")
            .as_deref(),
        Some("updated")
    );
    assert_eq!(db.l3_cache_size().await.expect("size2"), 1);
}

#[tokio::test]
async fn test_l3_cache_distinct_keys_dont_collide() {
    use ntk::metrics::MetricsDb;
    let tmp = tempfile::tempdir().expect("tempdir");
    let db = MetricsDb::init(&tmp.path().join("m.db"))
        .await
        .expect("init");

    let k1 = MetricsDb::l3_cache_key("a", "b", "m", "p");
    let k2 = MetricsDb::l3_cache_key("a", "c", "m", "p");
    db.store_l3_cache(&k1, "one", "m").await.expect("store1");
    db.store_l3_cache(&k2, "two", "m").await.expect("store2");
    assert_eq!(
        db.lookup_l3_cache(&k1, 30).await.expect("get1").as_deref(),
        Some("one")
    );
    assert_eq!(
        db.lookup_l3_cache(&k2, 30).await.expect("get2").as_deref(),
        Some("two")
    );
}

#[tokio::test]
async fn test_audit_log_silent_when_disabled() {
    // With audit_log=false (default), no file should be created.
    let tmp = tempfile::tempdir().expect("tempdir");
    let audit_path = tmp.path().join("audit.log");
    let mut config = ntk::config::NtkConfig::default();
    config.security.audit_log = false;
    config.security.audit_log_path = audit_path.to_string_lossy().into_owned();

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

    server
        .post("/compress")
        .json(&serde_json::json!({ "output": "some long output ".repeat(30) }))
        .await
        .assert_status_ok();

    assert!(
        !audit_path.exists(),
        "audit log file created despite audit_log=false"
    );
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
