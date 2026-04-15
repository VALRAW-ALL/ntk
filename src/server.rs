use crate::compressor::{layer1_filter, layer2_tokenizer, layer3_backend, layer4_context};
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
    /// Model info string — "phi3:mini q5_k_m [GPU]" — served via /state.
    pub model_info: String,
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
    /// Optional: Layer 4 — direct intent override. When set, this string is
    /// used as the user-intent prefix; `transcript_path` is ignored.
    #[serde(default)]
    pub context: Option<String>,
    /// Optional: Layer 4 — path to the Claude Code session .jsonl. When set,
    /// NTK reads the most recent user message to build an intent-aware prompt
    /// for L3. Ignored when `context` is already provided.
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// Optional: caller's current working directory (for metric annotation only).
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct LayerLatency {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l1: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l2: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l3: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CompressResponse {
    pub compressed: String,
    pub ratio: f32,
    pub layer: u8,
    pub tokens_before: usize,
    pub tokens_after: usize,
    /// Token count after Layer 1 alone (before L2 runs). `None` for passthrough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_after_l1: Option<usize>,
    /// Token count after Layer 2 (before L3 runs). `None` for passthrough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_after_l2: Option<usize>,
    /// Token count after Layer 3 (if triggered). `None` when L3 skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_after_l3: Option<usize>,
    /// Per-layer latency in milliseconds.
    #[serde(default, skip_serializing_if = "is_empty_latency")]
    pub latency_ms: LayerLatency,
}

fn is_empty_latency(l: &LayerLatency) -> bool {
    l.l1.is_none() && l.l2.is_none() && l.l3.is_none()
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
        .route("/records", get(handle_records))
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
            tokens_after_l1: None,
            tokens_after_l2: None,
            tokens_after_l3: None,
            latency_ms: LayerLatency::default(),
        }));
    }

    let started = Instant::now();

    // Count original tokens before any processing.
    let original_tokens = layer2_tokenizer::count_tokens(&req.output)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Layer 1 — with per-layer timing
    let l1_start = Instant::now();
    let l1 = layer1_filter::filter(&req.output);
    let l1_latency = l1_start.elapsed().as_millis() as u64;
    let tokens_after_l1 = layer2_tokenizer::count_tokens(&l1.output)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Layer 2
    let l2_start = Instant::now();
    let l2 = layer2_tokenizer::process(&l1.output)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let l2_latency = l2_start.elapsed().as_millis() as u64;
    let tokens_after_l2 = l2.compressed_tokens;

    let threshold = state.config.compression.inference_threshold_tokens;
    let output_type = detector::detect(&req.output);

    // Layer 4 — Context Injection.
    // Build an intent prefix from either the request's explicit `context`
    // field or by reading the Claude Code transcript at `transcript_path`.
    // Only active when the config flag is on.
    let context_prefix: String = if state.config.compression.context_aware {
        if let Some(direct) = req.context.as_deref() {
            if !direct.trim().is_empty() {
                layer4_context::format_context(
                    &layer4_context::SessionContext {
                        user_intent: direct.trim().to_owned(),
                        turns_ago: 0,
                    },
                    layer4_context::PromptFormat::default(),
                )
            } else {
                String::new()
            }
        } else if let Some(tpath) = req.transcript_path.as_deref() {
            let path = std::path::Path::new(tpath);
            match layer4_context::extract_context(path) {
                Some(ctx) => {
                    tracing::info!(
                        "Layer 4: extracted context from {} ({} turns ago): {}...",
                        path.display(),
                        ctx.turns_ago,
                        ctx.user_intent.chars().take(60).collect::<String>()
                    );
                    layer4_context::format_context(&ctx, layer4_context::PromptFormat::default())
                }
                None => String::new(),
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Layer 3 (optional, only when threshold exceeded)
    let mut l3_latency: Option<u64> = None;
    let mut tokens_after_l3: Option<usize> = None;
    let (output, layer_used, final_tokens) =
        if state.config.compression.layer3_enabled && l2.compressed_tokens > threshold {
            // Prompts dir: NTK_PROMPTS_DIR env var, or ~/.ntk/system-prompts/, or ./system-prompts/
            let prompts_dir = crate::config::resolve_prompts_dir();
            // Prepend the context prefix (empty string when L4 is off).
            let l3_input = if context_prefix.is_empty() {
                l2.output.clone()
            } else {
                format!("{context_prefix}{}", l2.output)
            };
            let l3_start = Instant::now();
            let l3_result = state
                .backend
                .compress(&l3_input, output_type, &prompts_dir)
                .await;
            l3_latency = Some(l3_start.elapsed().as_millis() as u64);
            match l3_result {
                Ok(l3) => {
                    let l3_tokens = layer2_tokenizer::count_tokens(&l3.output)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                    tokens_after_l3 = Some(l3_tokens);
                    (l3.output, 3u8, l3_tokens)
                }
                Err(e) => {
                    tracing::warn!(
                        "Layer 3 inference failed ({name}): {e}",
                        name = state.backend.name()
                    );
                    // Graceful fallback: Layer 3 unavailable → use Layer 2 output.
                    if state.config.model.fallback_to_layer1_on_timeout {
                        (l2.output.clone(), 2u8, l2.compressed_tokens)
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
            (l2.output.clone(), 2u8, l2.compressed_tokens)
        };

    let latency_ms = started.elapsed().as_millis() as u64;
    let compressed_tokens = final_tokens;

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

    let response = CompressResponse {
        compressed: output.clone(),
        ratio,
        layer: layer_used,
        tokens_before: original_tokens,
        tokens_after: compressed_tokens,
        tokens_after_l1: Some(tokens_after_l1),
        tokens_after_l2: Some(tokens_after_l2),
        tokens_after_l3,
        latency_ms: LayerLatency {
            l1: Some(l1_latency),
            l2: Some(l2_latency),
            l3: l3_latency,
        },
    };

    // Opt-in: persist the full compression trace to ~/.ntk/logs/ for
    // benchmarking / auditing when NTK_LOG_COMPRESSIONS=1 is set.
    if std::env::var("NTK_LOG_COMPRESSIONS").ok().as_deref() == Some("1") {
        let log_payload = CompressionLog {
            ts: Utc::now(),
            command: req.command.clone().unwrap_or_default(),
            cwd: req.context.clone().unwrap_or_default(),
            input: req.output.clone(),
            after_l1: l1.output.clone(),
            after_l2: l2.output.clone(),
            after_l3: if layer_used == 3 {
                Some(output.clone())
            } else {
                None
            },
            final_output: output,
            tokens: LogTokens {
                before: original_tokens,
                l1: tokens_after_l1,
                l2: tokens_after_l2,
                l3: tokens_after_l3,
            },
            latency_ms_total: latency_ms,
            latency_ms_l1: l1_latency,
            latency_ms_l2: l2_latency,
            latency_ms_l3: l3_latency,
            layer_used,
        };
        tokio::spawn(async move {
            if let Err(e) = write_compression_log(&log_payload) {
                tracing::warn!("failed to write compression log: {e}");
            }
        });
    }

    Ok(RespJson(response))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// GET /records — all in-memory compression records for the current session
// ---------------------------------------------------------------------------

async fn handle_records(
    State(state): State<AppState>,
) -> Result<RespJson<Vec<CompressionRecord>>, (StatusCode, String)> {
    let records = state
        .metrics
        .lock()
        .map(|m| m.recent(usize::MAX).to_vec())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(RespJson(records))
}

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
    model_info: String,
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
        model_info: state.model_info.clone(),
    }))
}

// ---------------------------------------------------------------------------
// Compression log — opt-in disk persistence for benchmarking (NTK_LOG_COMPRESSIONS=1)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct LogTokens {
    before: usize,
    l1: usize,
    l2: usize,
    l3: Option<usize>,
}

#[derive(Debug, Serialize)]
struct CompressionLog {
    ts: chrono::DateTime<Utc>,
    command: String,
    cwd: String,
    input: String,
    after_l1: String,
    after_l2: String,
    after_l3: Option<String>,
    #[serde(rename = "final")]
    final_output: String,
    tokens: LogTokens,
    latency_ms_total: u64,
    latency_ms_l1: u64,
    latency_ms_l2: u64,
    latency_ms_l3: Option<u64>,
    layer_used: u8,
}

fn write_compression_log(log: &CompressionLog) -> std::io::Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| std::io::Error::other("cannot determine home directory"))?;
    let day_dir = home
        .join(".ntk")
        .join("logs")
        .join(log.ts.format("%Y-%m-%d").to_string());
    std::fs::create_dir_all(&day_dir)?;

    // Collision-free filename without pulling in a uuid crate: timestamp + nanos.
    let stamp = log.ts.format("%H%M%S%3f").to_string();
    let file_path = day_dir.join(format!("{stamp}.json"));

    let json =
        serde_json::to_string_pretty(log).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(file_path, json)
}
