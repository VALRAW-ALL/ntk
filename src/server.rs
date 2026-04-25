use crate::compressor::{
    layer1_filter, layer2_tokenizer, layer3_backend, layer4_context, spec_loader,
};
use crate::config::NtkConfig;
use crate::detector;
use crate::metrics::{CompressionRecord, MetricsDb, MetricsStore};
use crate::output::dashboard::{WarnBuffer, WarnEntry};
use crate::security;
use axum::{
    extract::{Json, State},
    http::{Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json as RespJson, Redirect, Response},
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
    /// Layer 3 inference backend chain. A single-element chain mirrors
    /// the pre-#9 behavior; multi-element chains give fallback semantics.
    pub backend: Arc<layer3_backend::BackendChain>,
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
    /// Shared secret the hook must present on privileged routes. Empty
    /// when auth is intentionally disabled via `NTK_DISABLE_AUTH=1`.
    pub auth_token: Arc<String>,
    /// (Experimental, RFC-0001 POC) Pre-loaded YAML rulesets applied
    /// between L1 and L2 when `compression.spec_rules_path` is set
    /// (or `NTK_SPEC_RULES` env var overrides). Empty = feature off.
    /// Cached here so the hot path never re-parses YAML.
    pub spec_rules: Arc<Vec<spec_loader::RuleFile>>,
}

/// Resolve the effective spec-rules path from env var (wins) then
/// config. `NTK_SPEC_RULES` is the experimental override used by the
/// prompt/format A/B bench. Returns `None` when the feature is off.
pub fn resolve_spec_rules_path(config: &NtkConfig) -> Option<std::path::PathBuf> {
    if let Ok(v) = std::env::var("NTK_SPEC_RULES") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed));
        }
    }
    config.compression.spec_rules_path.clone()
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

/// Stable string label for a `PromptFormat` variant. Used as part of the
/// L3 cache key so switching prompt strategy invalidates cached rows
/// automatically.
fn prompt_format_label(f: layer4_context::PromptFormat) -> &'static str {
    match f {
        layer4_context::PromptFormat::Prefix => "prefix",
        layer4_context::PromptFormat::XmlWrap => "xmlwrap",
        layer4_context::PromptFormat::Goal => "goal",
        layer4_context::PromptFormat::Json => "json",
    }
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
    // /health stays open so `ntk status` and external liveness checks can
    // hit it without the token. Everything else — including /state which
    // exposes warn logs and config hints — requires the token.
    let protected = Router::new()
        .route("/compress", post(handle_compress))
        .route("/metrics", get(handle_metrics))
        .route("/records", get(handle_records))
        .route("/state", get(handle_state))
        .layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/health", get(handle_health))
        .route("/dashboard", get(handle_dashboard))
        .merge(protected)
        .fallback(redirect_to_dashboard)
        .with_state(state)
}

/// Catch-all for unmapped paths — sends the browser to /dashboard so
/// hitting `/` or a typo doesn't dead-end on a 404.
async fn redirect_to_dashboard() -> Redirect {
    Redirect::temporary("/dashboard")
}

/// Serves a self-contained HTML dashboard page. The page itself carries
/// no secrets — it prompts the user for the X-NTK-Token (same value
/// stored at `~/.ntk/.token`) and uses it from the browser to poll
/// `/metrics` every 5 s. Token stays in `sessionStorage` only; no cookies.
///
/// The route is unprotected so the user can load it without already
/// having the token in a header; the data behind it stays protected via
/// the existing /metrics gate.
async fn handle_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(DASHBOARD_HTML)
}

/// Inline single-page dashboard. Kept minimal: no build step, no
/// external JS/CSS — works offline, bypasses CSP on air-gapped hosts.
const DASHBOARD_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>NTK Dashboard</title>
<style>
  :root {
    --bg-0: #0a0e14;
    --bg-1: #0f1419;
    --bg-2: #161b22;
    --surface: rgba(124, 77, 255, 0.04);
    --surface-hi: rgba(124, 77, 255, 0.10);
    --border: rgba(124, 77, 255, 0.18);
    --border-hi: rgba(124, 77, 255, 0.35);
    --fg: #e6e8eb;
    --fg-dim: #b3b9c2;
    --muted: #6e7681;
    --accent: #7c4dff;
    --accent-hi: #9575ff;
    --ok: #4caf50;
    --warn: #ff9800;
    --err: #ef5350;
    --l1: #4caf50;
    --l2: #2196f3;
    --l3: #ff9800;
    --shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    --radius: 10px;
    --radius-sm: 6px;
    --mono: ui-monospace, 'JetBrains Mono', 'Cascadia Code', Menlo, Consolas, monospace;
    --sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  * { box-sizing: border-box; }

  html, body { height: 100%; }

  body {
    margin: 0;
    font-family: var(--sans);
    background: radial-gradient(ellipse at top, #1a1530 0%, var(--bg-0) 60%);
    background-attachment: fixed;
    color: var(--fg);
    line-height: 1.5;
    min-height: 100vh;
    -webkit-font-smoothing: antialiased;
  }

  .shell {
    max-width: 1200px;
    margin: 0 auto;
    padding: clamp(1rem, 3vw, 2.5rem) clamp(1rem, 4vw, 2rem);
  }

  /* ---------- Header ---------- */
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 1rem;
    margin-bottom: 2rem;
    padding-bottom: 1.25rem;
    border-bottom: 1px solid var(--border);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.85rem;
  }
  .logo {
    width: 38px; height: 38px;
    border-radius: 9px;
    background: linear-gradient(135deg, var(--accent) 0%, #4d2bff 100%);
    display: flex; align-items: center; justify-content: center;
    font-family: var(--mono);
    font-weight: 700;
    color: white;
    font-size: 0.9rem;
    box-shadow: 0 4px 14px rgba(124, 77, 255, 0.4);
  }
  .title-block h1 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .title-block .sub {
    font-size: 0.78rem;
    color: var(--muted);
    margin-top: 1px;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.85rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 999px;
    font-family: var(--mono);
    font-size: 0.75rem;
    color: var(--fg-dim);
  }
  .status-dot {
    width: 8px; height: 8px;
    border-radius: 50%;
    background: var(--ok);
    box-shadow: 0 0 0 3px rgba(76, 175, 80, 0.18);
    animation: pulse 2.4s ease-in-out infinite;
  }
  .status-dot.err { background: var(--err); box-shadow: 0 0 0 3px rgba(239, 83, 80, 0.18); animation: none; }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }

  /* ---------- KPI cards ---------- */
  .kpi-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
  }
  .kpi {
    background: linear-gradient(180deg, var(--surface-hi) 0%, var(--surface) 100%);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1.25rem 1.4rem;
    transition: border-color 0.2s, transform 0.2s;
    position: relative;
    overflow: hidden;
  }
  .kpi:hover { border-color: var(--border-hi); transform: translateY(-2px); }
  .kpi::before {
    content: '';
    position: absolute;
    top: 0; left: 0; right: 0;
    height: 2px;
    background: linear-gradient(90deg, var(--accent), transparent);
    opacity: 0.6;
  }
  .kpi .label {
    font-size: 0.72rem;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 500;
  }
  .kpi .value {
    font-family: var(--mono);
    font-size: clamp(1.5rem, 3vw, 1.9rem);
    font-weight: 600;
    margin-top: 0.45rem;
    color: var(--fg);
    letter-spacing: -0.02em;
  }
  .kpi .sub {
    font-size: 0.78rem;
    color: var(--muted);
    margin-top: 0.4rem;
  }

  /* ---------- Sections ---------- */
  section {
    background: var(--bg-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1.5rem;
    margin-bottom: 1.25rem;
    box-shadow: var(--shadow);
  }
  section h2 {
    font-size: 0.9rem;
    font-weight: 600;
    margin: 0 0 1rem;
    color: var(--accent-hi);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  /* ---------- Layer bar ---------- */
  .layer-bar {
    display: flex;
    height: 28px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.05);
  }
  .layer-bar span {
    display: flex; align-items: center; justify-content: center;
    font-family: var(--mono);
    font-size: 0.72rem;
    color: white;
    font-weight: 500;
    transition: filter 0.2s;
  }
  .layer-bar span:hover { filter: brightness(1.15); }
  .layer-bar .l1 { background: linear-gradient(180deg, #5fcc63 0%, var(--l1) 100%); }
  .layer-bar .l2 { background: linear-gradient(180deg, #42a5f5 0%, var(--l2) 100%); }
  .layer-bar .l3 { background: linear-gradient(180deg, #ffa726 0%, var(--l3) 100%); }
  .layer-legend {
    display: flex;
    gap: 1.25rem;
    margin-top: 0.85rem;
    font-family: var(--mono);
    font-size: 0.78rem;
    color: var(--fg-dim);
    flex-wrap: wrap;
  }
  .layer-legend .swatch {
    display: inline-block;
    width: 10px; height: 10px;
    border-radius: 2px;
    margin-right: 0.4rem;
    vertical-align: middle;
  }

  /* ---------- Footer ---------- */
  footer {
    text-align: center;
    color: var(--muted);
    font-size: 0.75rem;
    margin-top: 2rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
  }
  footer a { color: var(--accent-hi); text-decoration: none; }
  footer a:hover { text-decoration: underline; }

  /* ---------- Auth screen ---------- */
  .auth-wrap {
    min-height: calc(100vh - 6rem);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .auth-card {
    width: 100%;
    max-width: 420px;
    background: var(--bg-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 2rem;
    box-shadow: var(--shadow);
    text-align: center;
  }
  .auth-card .lock {
    width: 56px; height: 56px;
    margin: 0 auto 1rem;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--accent) 0%, #4d2bff 100%);
    display: flex; align-items: center; justify-content: center;
    font-size: 1.4rem;
    box-shadow: 0 6px 20px rgba(124, 77, 255, 0.35);
  }
  .auth-card h2 {
    font-size: 1.05rem;
    margin: 0 0 0.4rem;
    color: var(--fg);
    text-transform: none;
    letter-spacing: 0;
  }
  .auth-card p {
    color: var(--fg-dim);
    font-size: 0.85rem;
    margin: 0 0 1.5rem;
  }
  .auth-card code {
    background: rgba(255, 255, 255, 0.06);
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    font-family: var(--mono);
    font-size: 0.8rem;
  }
  .input {
    width: 100%;
    padding: 0.75rem 0.95rem;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg);
    font-family: var(--mono);
    font-size: 0.85rem;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(124, 77, 255, 0.15);
  }
  .btn {
    width: 100%;
    margin-top: 0.75rem;
    padding: 0.75rem 1rem;
    background: linear-gradient(180deg, var(--accent-hi) 0%, var(--accent) 100%);
    border: none;
    border-radius: var(--radius-sm);
    color: white;
    cursor: pointer;
    font-family: var(--sans);
    font-size: 0.9rem;
    font-weight: 600;
    letter-spacing: 0.01em;
    transition: filter 0.15s, transform 0.05s;
  }
  .btn:hover { filter: brightness(1.1); }
  .btn:active { transform: translateY(1px); }
  .error {
    color: var(--err);
    background: rgba(239, 83, 80, 0.08);
    border: 1px solid rgba(239, 83, 80, 0.25);
    border-radius: var(--radius-sm);
    padding: 0.6rem 0.8rem;
    margin-top: 0.85rem;
    font-size: 0.82rem;
  }

  /* ---------- Skeleton ---------- */
  .skeleton {
    background: linear-gradient(90deg, var(--surface) 0%, var(--surface-hi) 50%, var(--surface) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.5s infinite linear;
    border-radius: var(--radius-sm);
  }
  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  /* ---------- Responsive tweaks ---------- */
  @media (max-width: 600px) {
    header { flex-direction: column; align-items: flex-start; }
    .kpi { padding: 1rem 1.1rem; }
    section { padding: 1.1rem; }
  }
</style>
</head>
<body>
  <div class="shell">
    <header>
      <div class="brand">
        <div class="logo">NTK</div>
        <div class="title-block">
          <h1>Neural Token Killer</h1>
          <div class="sub">live compression dashboard</div>
        </div>
      </div>
      <div class="status-pill" id="status">
        <span class="status-dot" id="dot"></span>
        <span id="status-text">connecting…</span>
      </div>
    </header>

    <main id="content">
      <div class="kpi-grid">
        <div class="kpi"><div class="label">loading</div><div class="value skeleton" style="height: 1.9rem; width: 60%;">&nbsp;</div></div>
        <div class="kpi"><div class="label">loading</div><div class="value skeleton" style="height: 1.9rem; width: 60%;">&nbsp;</div></div>
        <div class="kpi"><div class="label">loading</div><div class="value skeleton" style="height: 1.9rem; width: 60%;">&nbsp;</div></div>
        <div class="kpi"><div class="label">loading</div><div class="value skeleton" style="height: 1.9rem; width: 60%;">&nbsp;</div></div>
      </div>
    </main>

    <footer>
      polling every 5 s · <a href="https://github.com/VALRAW-ALL/ntk" target="_blank" rel="noopener">github.com/VALRAW-ALL/ntk</a>
    </footer>
  </div>

  <script>
    const KEY = 'ntk_token';
    let token = sessionStorage.getItem(KEY);
    const content = document.getElementById('content');
    const dot = document.getElementById('dot');
    const statusText = document.getElementById('status-text');

    function setStatus(text, ok = true) {
      statusText.textContent = text;
      dot.classList.toggle('err', !ok);
    }

    function renderTokenForm(err) {
      content.innerHTML = `
        <div class="auth-wrap">
          <div class="auth-card">
            <div class="lock">🔒</div>
            <h2>Authentication required</h2>
            <p>Paste the daemon's shared secret from <code>~/.ntk/.token</code></p>
            <input id="tok" class="input" placeholder="X-NTK-Token" autocomplete="off" autofocus />
            <button id="save" class="btn">Connect</button>
            ${err ? `<div class="error">${err}</div>` : ''}
          </div>
        </div>`;
      const submit = () => {
        const v = document.getElementById('tok').value.trim();
        if (v) { sessionStorage.setItem(KEY, v); token = v; poll(); }
      };
      document.getElementById('save').onclick = submit;
      document.getElementById('tok').addEventListener('keydown', e => {
        if (e.key === 'Enter') submit();
      });
    }

    async function poll() {
      if (!token) { setStatus('awaiting token', false); return renderTokenForm(); }
      try {
        const r = await fetch('/metrics', { headers: { 'X-NTK-Token': token } });
        if (r.status === 401) {
          sessionStorage.removeItem(KEY);
          token = null;
          setStatus('invalid token', false);
          return renderTokenForm('Invalid token. Check ~/.ntk/.token and try again.');
        }
        if (!r.ok) throw new Error('HTTP ' + r.status);
        const d = await r.json();
        setStatus(`live · last sync ${new Date().toLocaleTimeString()}`, true);
        render(d);
      } catch (e) {
        setStatus('error: ' + e.message, false);
      }
    }

    function render(d) {
      const total = d.total_compressions || 0;
      const saved = d.total_tokens_saved || 0;
      const avg = ((d.average_ratio || 0) * 100).toFixed(1);
      const rtk = d.rtk_pre_filtered_count || 0;
      const layers = d.layer_counts || [0, 0, 0];
      const layerTotal = layers[0] + layers[1] + layers[2] || 1;
      const pct = i => Math.round((layers[i] / layerTotal) * 100);

      const savedL1 = d.total_saved_l1 || 0;
      const savedL2 = d.total_saved_l2 || 0;
      const savedL3 = d.total_saved_l3 || 0;
      const savedTotal = savedL1 + savedL2 + savedL3 || 1;
      const savedPct = v => Math.round((v / savedTotal) * 100);

      content.innerHTML = `
        <div class="kpi-grid">
          <div class="kpi">
            <div class="label">Compressions</div>
            <div class="value">${total.toLocaleString()}</div>
            <div class="sub">total runs</div>
          </div>
          <div class="kpi">
            <div class="label">Tokens Saved</div>
            <div class="value">${saved.toLocaleString()}</div>
            <div class="sub">across all sessions</div>
          </div>
          <div class="kpi">
            <div class="label">Avg Ratio</div>
            <div class="value">${avg}%</div>
            <div class="sub">compression efficiency</div>
          </div>
          <div class="kpi">
            <div class="label">RTK Pre-filtered</div>
            <div class="value">${rtk.toLocaleString()}</div>
            <div class="sub">hook ran after RTK</div>
          </div>
        </div>
        <section>
          <h2>Layer Distribution <small style="font-weight:400;opacity:.6">(by winning layer)</small></h2>
          <div class="layer-bar">
            ${pct(0) > 0 ? `<span class="l1" style="width:${pct(0)}%" title="L1: fast filter">L1 ${pct(0)}%</span>` : ''}
            ${pct(1) > 0 ? `<span class="l2" style="width:${pct(1)}%" title="L2: tokenizer-aware">L2 ${pct(1)}%</span>` : ''}
            ${pct(2) > 0 ? `<span class="l3" style="width:${pct(2)}%" title="L3: neural inference">L3 ${pct(2)}%</span>` : ''}
          </div>
          <div class="layer-legend">
            <span><span class="swatch" style="background: var(--l1)"></span>L1 fast filter · ${layers[0].toLocaleString()}</span>
            <span><span class="swatch" style="background: var(--l2)"></span>L2 tokenizer · ${layers[1].toLocaleString()}</span>
            <span><span class="swatch" style="background: var(--l3)"></span>L3 inference · ${layers[2].toLocaleString()}</span>
          </div>
        </section>
        <section>
          <h2>Tokens Saved by Layer <small style="font-weight:400;opacity:.6">(incremental contribution)</small></h2>
          <div class="layer-bar">
            ${savedPct(savedL1) > 0 ? `<span class="l1" style="width:${savedPct(savedL1)}%" title="L1 incremental savings">L1 ${savedPct(savedL1)}%</span>` : ''}
            ${savedPct(savedL2) > 0 ? `<span class="l2" style="width:${savedPct(savedL2)}%" title="L2 incremental savings">L2 ${savedPct(savedL2)}%</span>` : ''}
            ${savedPct(savedL3) > 0 ? `<span class="l3" style="width:${savedPct(savedL3)}%" title="L3 incremental savings">L3 ${savedPct(savedL3)}%</span>` : ''}
          </div>
          <div class="layer-legend">
            <span><span class="swatch" style="background: var(--l1)"></span>L1 · ${savedL1.toLocaleString()} tokens</span>
            <span><span class="swatch" style="background: var(--l2)"></span>L2 · ${savedL2.toLocaleString()} tokens</span>
            <span><span class="swatch" style="background: var(--l3)"></span>L3 · ${savedL3.toLocaleString()} tokens</span>
          </div>
        </section>`;
    }

    poll();
    setInterval(poll, 5000);
  </script>
</body>
</html>
"##;

/// Rejects requests to privileged routes that lack a matching
/// `X-NTK-Token` header. Bypass only via `NTK_DISABLE_AUTH=1`.
async fn require_token(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    if security::auth_disabled() {
        return Ok(next.run(req).await);
    }
    let expected = state.auth_token.as_str();
    if expected.is_empty() {
        // No token configured — treat as auth disabled but log a warning.
        // This branch is defensive; startup should always populate a token.
        tracing::warn!(
            "auth_token is empty — permitting request. Restart `ntk start` to re-generate."
        );
        return Ok(next.run(req).await);
    }
    let presented = req
        .headers()
        .get(security::TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if security::constant_time_eq(expected, presented) {
        return Ok(next.run(req).await);
    }
    // Browser-style GET with no token at all → bounce to /dashboard so the
    // user lands on the auth-prompt UI instead of a raw 401 page.
    // Anything with a (wrong) token, or any non-GET method (the dashboard
    // JS / hooks), still gets a clean 401 so callers can react.
    if req.method() == Method::GET && presented.is_empty() {
        return Ok(Redirect::temporary("/dashboard").into_response());
    }
    Err((StatusCode::UNAUTHORIZED, "missing or invalid X-NTK-Token"))
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

    // L1.5 — experimental spec-loader stage (RFC-0001 POC, #24).
    // No-op when `state.spec_rules` is empty. The spec-loader's own
    // `preserve_errors` invariant drops any rule whose transform
    // would lose error signal, so worst-case this stage is a pass-
    // through — never a regression versus pre-#24 behaviour.
    let l1_output = if state.spec_rules.is_empty() {
        l1.output.clone()
    } else {
        let r = spec_loader::apply_many(&l1.output, &state.spec_rules);
        if !r.invariant_rejected.is_empty() {
            tracing::debug!(
                "spec_rules: {} rule(s) rejected by invariants: {:?}",
                r.invariant_rejected.len(),
                r.invariant_rejected
            );
        }
        r.output
    };

    // Layer 2
    let l2_start = Instant::now();
    let l2 = layer2_tokenizer::process(&l1_output)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let l2_latency = l2_start.elapsed().as_millis() as u64;
    let tokens_after_l2 = l2.compressed_tokens;

    let threshold = state.config.compression.inference_threshold_tokens;
    let output_type = detector::detect(&req.output);

    // Layer 4 — Context Injection.
    // Build an intent prefix from either the request's explicit `context`
    // field or by reading the Claude Code transcript at `transcript_path`.
    // Only active when the config flag is on.
    //
    // Prompt format can be overridden at runtime via NTK_L4_FORMAT=prefix|xml|
    // goal|json — used by the bench/prompt_formats.ps1 A/B experiment.
    let prompt_format = match std::env::var("NTK_L4_FORMAT").ok().as_deref() {
        Some("xml") | Some("xmlwrap") => layer4_context::PromptFormat::XmlWrap,
        Some("goal") => layer4_context::PromptFormat::Goal,
        Some("json") => layer4_context::PromptFormat::Json,
        _ => layer4_context::PromptFormat::default(),
    };
    let context_prefix: String = if state.config.compression.context_aware {
        if let Some(direct) = req.context.as_deref() {
            if !direct.trim().is_empty() {
                layer4_context::format_context(
                    &layer4_context::SessionContext {
                        user_intent: direct.trim().to_owned(),
                        turns_ago: 0,
                    },
                    prompt_format,
                )
            } else {
                String::new()
            }
        } else if let Some(tpath) = req.transcript_path.as_deref() {
            let path = std::path::Path::new(tpath);
            let max_msgs = state.config.compression.context_max_messages;
            match layer4_context::extract_context_with_decay(path, max_msgs) {
                Some(ctx) => {
                    tracing::info!(
                        "Layer 4: extracted context from {} ({} turns ago): {}...",
                        path.display(),
                        ctx.turns_ago,
                        ctx.user_intent.chars().take(60).collect::<String>()
                    );
                    layer4_context::format_context(&ctx, prompt_format)
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
    let mut l3_cache_hit = false;
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

            // L3 cache: identical (l2_output, context, model, format) →
            // identical output, every time. Checking SQLite (~1 ms) is
            // strictly cheaper than re-running inference (50–800 ms).
            let cache_enabled = state.config.l3_cache.enabled;
            let backend_name = state.backend.name().to_owned();
            let prompt_format_key = prompt_format_label(prompt_format);
            let cache_key = MetricsDb::l3_cache_key(
                &l2.output,
                &context_prefix,
                &backend_name,
                prompt_format_key,
            );
            let cached_output = if cache_enabled {
                if let Some(db) = state.db.as_ref() {
                    match db
                        .lookup_l3_cache(&cache_key, state.config.l3_cache.ttl_days)
                        .await
                    {
                        Ok(hit) => hit,
                        Err(e) => {
                            tracing::warn!("l3_cache lookup failed: {e}");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(hit) = cached_output {
                l3_cache_hit = true;
                l3_latency = Some(0);
                let l3_tokens = layer2_tokenizer::count_tokens(&hit)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                tokens_after_l3 = Some(l3_tokens);
                (hit, 3u8, l3_tokens)
            } else {
                let l3_start = Instant::now();
                let l3_result = state
                    .backend
                    .compress(&l3_input, output_type, &prompts_dir)
                    .await;
                l3_latency = Some(l3_start.elapsed().as_millis() as u64);
                match l3_result {
                    Ok((l3, backend_used)) => {
                        let l3_tokens = layer2_tokenizer::count_tokens(&l3.output)
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                        tokens_after_l3 = Some(l3_tokens);

                        if backend_used != backend_name {
                            tracing::info!(
                            "L3 fallback: primary '{backend_name}' failed, used '{backend_used}'"
                        );
                        }

                        // Cache the successful completion for reuse on
                        // identical inputs. Best-effort — storage errors
                        // are logged but don't affect the response. The
                        // cache key uses the actually-used backend name so
                        // a fallback hit does not poison the primary's row.
                        if cache_enabled {
                            if let Some(db) = state.db.as_ref() {
                                let fallback_key = MetricsDb::l3_cache_key(
                                    &l2.output,
                                    &context_prefix,
                                    backend_used,
                                    prompt_format_key,
                                );
                                if let Err(e) = db
                                    .store_l3_cache(&fallback_key, &l3.output, backend_used)
                                    .await
                                {
                                    tracing::warn!("l3_cache store failed: {e}");
                                }
                            }
                        }

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
            }
        } else {
            (l2.output.clone(), 2u8, l2.compressed_tokens)
        };
    // Tracing: signal cache hits so operators can tune ttl_days / size.
    if l3_cache_hit {
        tracing::info!(
            "L3 cache hit: {tokens}t in 0ms (original L3 would be ~{threshold}+ ms)",
            tokens = final_tokens,
            threshold = threshold
        );
    }

    let latency_ms = started.elapsed().as_millis() as u64;
    let compressed_tokens = final_tokens;

    let ratio = if original_tokens == 0 {
        0.0
    } else {
        let saved = original_tokens.saturating_sub(compressed_tokens);
        saved as f32 / original_tokens as f32
    };

    // Re-attribute `layer_used` from "highest stage reached" (always 2 or 3
    // because L1 can never be the terminal stage) to "stage that contributed
    // the most savings". This is what users actually want from the
    // dashboard's Layer Distribution chart — without it L1 always shows 0.
    let l1_saved = original_tokens.saturating_sub(tokens_after_l1);
    let l2_saved = tokens_after_l1.saturating_sub(tokens_after_l2);
    let l3_saved = tokens_after_l2.saturating_sub(compressed_tokens);
    let winning_layer = if layer_used == 3 && l3_saved >= l1_saved && l3_saved >= l2_saved {
        3u8
    } else if l1_saved > l2_saved {
        1u8
    } else {
        2u8
    };

    // Record metrics.
    let record = CompressionRecord {
        command,
        output_type,
        original_tokens,
        compressed_tokens,
        tokens_after_l1: Some(tokens_after_l1),
        tokens_after_l2: Some(tokens_after_l2),
        layer_used: winning_layer,
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
        layer: winning_layer,
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

    // Opt-in audit log. When enabled, appends one JSONL line per request
    // with a SHA-256 of the output — never the output itself. Best-effort;
    // any I/O error is logged to tracing but does not fail the request.
    if state.config.security.audit_log {
        let path = crate::config::expand_tilde(&state.config.security.audit_log_path);
        let cmd = req.command.clone().unwrap_or_default();
        let cwd = req.cwd.clone().unwrap_or_default();
        let record = security::AuditRecord::new(
            &cmd,
            &cwd,
            original_tokens,
            compressed_tokens,
            winning_layer,
            &output,
        );
        security::append_audit_record(&path, &record);
    }

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
            after_l3: if tokens_after_l3.is_some() {
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
            layer_used: winning_layer,
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
