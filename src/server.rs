use crate::compressor::{layer1_filter, layer2_tokenizer, layer3_backend};
use crate::config::NtkConfig;
use crate::detector;
use crate::metrics::{CompressionRecord, MetricsDb, MetricsStore};
use crate::output::dashboard::{WarnBuffer, WarnEntry};
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::Json as RespJson,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Shared app state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<NtkConfig>,
    pub metrics: Arc<Mutex<MetricsStore>>,
    /// Optional SQLite persistence — None when db init fails or is disabled.
    pub db: Option<Arc<MetricsDb>>,
    /// Layer 3 inference backend (Ollama / Candle / llama.cpp).
    pub backend: Arc<layer3_backend::BackendKind>,
    /// Daemon start time — used to compute uptime in /health and /state.
    pub started_at: std::time::Instant,
    /// Captured WARN/ERROR log — served via /state for attach-mode TUI.
    pub warn_log: WarnBuffer,
    /// Bound address string (e.g. "127.0.0.1:8765") — served via /state.
    pub addr: String,
    /// GPU backend name — served via /state.
    pub backend_name: String,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CompressRequest {
    pub output: String,
    /// Optional: the Bash command that produced this output (used for metrics).
    #[serde(default)]
    pub command: Option<String>,
    /// Optional: Claude's current intent (Layer 4, not yet implemented).
    #[serde(default)]
    pub context: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CompressResponse {
    pub compressed: String,
    pub ratio: f32,
    pub layer: u8,
    pub tokens_before: usize,
    pub tokens_after: usize,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub model: String,
    pub uptime_secs: u64,
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/compress", post(handle_compress))
        .route("/metrics", get(handle_metrics))
        .route("/health", get(handle_health))
        .route("/state", get(handle_state))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// POST /compress
// ---------------------------------------------------------------------------

async fn handle_compress(
    State(state): State<AppState>,
    Json(req): Json<CompressRequest>,
) -> Result<RespJson<CompressResponse>, (StatusCode, String)> {
    // Security: enforce max input size.
    let max_chars = state.config.exclusions.max_input_chars;
    if req.output.len() > max_chars {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("output exceeds max_input_chars limit ({max_chars})"),
        ));
    }

    // Skip excluded commands.
    let command = req.command.clone().unwrap_or_default();
    let cmd_base = command.split_whitespace().next().unwrap_or("").to_owned();
    if state.config.exclusions.commands.contains(&cmd_base) {
        let tokens = layer2_tokenizer::count_tokens(&req.output)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(RespJson(CompressResponse {
            compressed: req.output,
            ratio: 0.0,
            layer: 0,
            tokens_before: tokens,
            tokens_after: tokens,
        }));
    }

    let started = Instant::now();

    // Count original tokens before any processing.
    let original_tokens = layer2_tokenizer::count_tokens(&req.output)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Layer 1
    let l1 = layer1_filter::filter(&req.output);

    // Layer 2
    let l2 = layer2_tokenizer::process(&l1.output)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let threshold = state.config.compression.inference_threshold_tokens;
    let output_type = detector::detect(&req.output);

    let (output, layer_used) =
        if state.config.compression.layer3_enabled && l2.compressed_tokens > threshold {
            // Prompts dir: NTK_PROMPTS_DIR env var, or ~/.ntk/system-prompts/, or ./system-prompts/
            let prompts_dir = crate::config::resolve_prompts_dir();
            match state
                .backend
                .compress(&l2.output, output_type, &prompts_dir)
                .await
            {
                Ok(l3) => (l3.output, 3u8),
                Err(e) => {
                    tracing::warn!(
                        "Layer 3 inference failed ({name}): {e}",
                        name = state.backend.name()
                    );
                    // Graceful fallback: Layer 3 unavailable → use Layer 2 output.
                    if state.config.model.fallback_to_layer1_on_timeout {
                        (l2.output.clone(), 2u8)
                    } else {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            format!(
                                "Layer 3 inference unavailable ({}): {e}",
                                state.backend.name()
                            ),
                        ));
                    }
                }
            }
        } else {
            (l2.output.clone(), 2u8)
        };

    let latency_ms = started.elapsed().as_millis() as u64;
    let compressed_tokens = l2.compressed_tokens;

    let ratio = if original_tokens == 0 {
        0.0
    } else {
        let saved = original_tokens.saturating_sub(compressed_tokens);
        saved as f32 / original_tokens as f32
    };

    // Record metrics.
    let record = CompressionRecord {
        command,
        output_type,
        original_tokens,
        compressed_tokens,
        layer_used,
        latency_ms,
        rtk_pre_filtered: l1.rtk_pre_filtered,
        timestamp: Utc::now(),
    };
    if let Ok(mut m) = state.metrics.lock() {
        m.record(record.clone());
    }

    // Persist to SQLite asynchronously (fire-and-forget — never blocks the response).
    if let Some(db) = &state.db {
        let db = db.clone();
        tokio::spawn(async move {
            if let Err(e) = db.persist(&record).await {
                tracing::warn!("failed to persist metrics to SQLite: {e}");
            }
        });
    }

    Ok(RespJson(CompressResponse {
        compressed: output,
        ratio,
        layer: layer_used,
        tokens_before: original_tokens,
        tokens_after: compressed_tokens,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// GET /metrics
// ---------------------------------------------------------------------------

async fn handle_metrics(
    State(state): State<AppState>,
) -> Result<RespJson<serde_json::Value>, (StatusCode, String)> {
    let summary = state
        .metrics
        .lock()
        .map(|m| m.session_summary())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(RespJson(serde_json::to_value(summary).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

async fn handle_health(State(state): State<AppState>) -> RespJson<HealthResponse> {
    RespJson(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        model: format!(
            "{} ({})",
            state.config.model.model_name,
            state.backend.name()
        ),
        uptime_secs: state.started_at.elapsed().as_secs(),
    })
}

// ---------------------------------------------------------------------------
// GET /state  — full dashboard snapshot for attach-mode TUI
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct StateResponse {
    summary: crate::metrics::SessionSummary,
    recent: Vec<CompressionRecord>,
    warns: Vec<WarnEntry>,
    uptime_secs: u64,
    addr: String,
    backend_name: String,
}

async fn handle_state(
    State(state): State<AppState>,
) -> Result<RespJson<StateResponse>, (StatusCode, String)> {
    let (summary, recent) = state
        .metrics
        .lock()
        .map(|m| (m.session_summary(), m.recent(3).to_vec()))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let warns = state
        .warn_log
        .lock()
        .map(|b| b.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    Ok(RespJson(StateResponse {
        summary,
        recent,
        warns,
        uptime_secs: state.started_at.elapsed().as_secs(),
        addr: state.addr.clone(),
        backend_name: state.backend_name.clone(),
    }))
}
