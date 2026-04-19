use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Sub-structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub port: u16,
    pub host: String,
    pub auto_start: bool,
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: 8765,
            host: "127.0.0.1".to_string(),
            auto_start: true,
            log_level: "warn".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionConfig {
    pub enabled: bool,
    pub layer1_enabled: bool,
    pub layer2_enabled: bool,
    pub layer3_enabled: bool,
    pub inference_threshold_tokens: usize,
    pub context_aware: bool,
    pub max_output_tokens: usize,
    pub preserve_first_stacktrace: bool,
    pub preserve_error_counts: bool,
    /// How many of the most-recent user messages Layer 4 folds into the
    /// context prefix. 1 keeps the original single-message behavior;
    /// larger values enable decay-weighted stacking of older messages to
    /// preserve long-running debug intent across many Bash calls.
    /// Internally bounded to 5 so the prefix stays below MAX_INTENT_CHARS
    /// even in the worst case.
    pub context_max_messages: usize,

    /// Which BPE tokenizer Layer 2 uses for token counting. Accepts:
    /// `"cl100k_base"` (default; Claude 3.x, GPT-3.5/4) or `"o200k_base"`
    /// (Claude 3.5+ / 4, GPT-4o / o1 family). The two differ by ~5-10 %
    /// on code-heavy outputs; pick the one your LLM uses so
    /// `tokens_before/after` reflect the real cost.
    #[serde(default = "default_tokenizer")]
    pub tokenizer: String,
}

fn default_tokenizer() -> String {
    "cl100k_base".to_string()
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            layer1_enabled: true,
            layer2_enabled: true,
            layer3_enabled: true,
            inference_threshold_tokens: 300,
            // Layer 4 context injection is on by default now that the
            // hook passes transcript_path to the daemon on every call.
            // Disable this to restore pre-v0.2.27 behaviour.
            context_aware: true,
            max_output_tokens: 500,
            preserve_first_stacktrace: true,
            preserve_error_counts: true,
            context_max_messages: 3,
            tokenizer: default_tokenizer(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    #[default]
    Ollama,
    Candle,
    LlamaCpp,
}

impl ModelProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::Candle => "candle",
            Self::LlamaCpp => "llama.cpp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub provider: ModelProvider,
    pub model_name: String,
    pub quantization: String,
    pub ollama_url: String,
    /// Upper bound for a single /compress call including Layer 3 inference.
    /// Large fixtures (> 1K tokens) on CPU-only Ollama/llama.cpp can take
    /// 60-180 s; default is 300 000 ms (5 min) so most real-world contexts
    /// complete before falling back to L2. Scale down for GPU setups.
    pub timeout_ms: u64,
    pub fallback_to_layer1_on_timeout: bool,
    pub temperature: f32,
    pub gpu_layers: i32,
    pub gpu_auto_detect: bool,
    pub cuda_device: u32,
    /// GPU vendor the user explicitly picked in `ntk model setup`.
    /// `None` = no choice made → fall back to auto-detection.
    /// Set explicitly, it overrides the detect_best_backend priority so picking
    /// an AMD card on a machine that also has NVIDIA actually routes inference
    /// to the AMD card instead of silently switching back to CUDA.
    #[serde(default)]
    pub gpu_vendor: Option<crate::gpu::GpuVendor>,
    pub llama_cpp_path: Option<PathBuf>,
    /// Path to the GGUF model file (Candle and llama.cpp backends).
    pub model_path: Option<PathBuf>,
    /// Path to the HuggingFace tokenizer.json (Candle backend).
    pub tokenizer_path: Option<PathBuf>,
    /// Port for the llama-server subprocess (llama.cpp backend). Default: 8766.
    pub llama_server_port: u16,
    /// Auto-start llama-server at daemon startup. Default: true.
    pub llama_server_auto_start: bool,
    /// Milliseconds to wait for llama-server to become healthy after spawning.
    /// Loading a 2.2 GB GGUF model on CPU can take 30-60 s. Default: 60 000.
    pub llama_server_start_timeout_ms: u64,

    /// Ordered fallback chain of backends. When the primary fails with an
    /// error (not just a timeout-caught fallback to L1+L2), NTK tries
    /// each subsequent backend in order before giving up. Empty vec
    /// (default) means "just use `provider`" — backward compatible.
    ///
    /// Example: `["ollama", "candle"]` uses Ollama as primary, Candle as
    /// fallback when Ollama is unreachable.
    #[serde(default)]
    pub backend_chain: Vec<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: ModelProvider::Ollama,
            model_name: "phi3:mini".to_string(),
            quantization: "q5_k_m".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            timeout_ms: 300_000,
            fallback_to_layer1_on_timeout: true,
            temperature: 0.1,
            gpu_layers: -1,
            gpu_auto_detect: true,
            cuda_device: 0,
            gpu_vendor: None,
            llama_cpp_path: None,
            model_path: None,
            tokenizer_path: None,
            backend_chain: Vec::new(),
            llama_server_port: 8766,
            llama_server_auto_start: true,
            llama_server_start_timeout_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub storage_path: String,
    pub history_days: u32,
    pub track_per_command: bool,
    pub track_per_session: bool,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_path: "~/.ntk/metrics.db".to_string(),
            history_days: 30,
            track_per_command: true,
            track_per_session: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExclusionsConfig {
    pub commands: Vec<String>,
    pub max_input_chars: usize,
}

impl Default for ExclusionsConfig {
    fn default() -> Self {
        Self {
            commands: vec!["cat".to_string(), "echo".to_string(), "pwd".to_string()],
            max_input_chars: 100_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub show_compression_ratio: bool,
    pub show_layer_used: bool,
    pub show_backend: bool,
    pub color: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            show_compression_ratio: true,
            show_layer_used: false,
            show_backend: true,
            color: true,
        }
    }
}

/// Deprecated. The telemetry feature was removed in #19 — NTK no longer
/// collects or sends any usage data. This struct is kept in the config
/// schema so existing `~/.ntk/config.json` files continue to parse (the
/// field is ignored at runtime). Safe to delete from your config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Ignored. Retained for schema-backward-compat only.
    #[serde(default)]
    pub enabled: bool,
}

/// Deterministic cache for Layer 3 inference results.
///
/// Running the same command twice in the same branch produces identical
/// output, which currently triggers a fresh L3 call (50–800 ms). Caching
/// the prompt→output pair in SQLite reduces repeat latency to a lookup
/// (<5 ms on a warm pool).
///
/// Cache key is `SHA-256(l2_output + context_prefix + model_name +
/// prompt_format)`. Entries older than `ttl_days` are pruned on lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct L3CacheConfig {
    pub enabled: bool,
    pub ttl_days: u32,
}

impl Default for L3CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_days: 7,
        }
    }
}

/// Optional security features (opt-in).
///
/// `audit_log` appends a single JSONL record per /compress call to
/// `audit_log_path`. Records carry a timestamp, the originating command
/// name (no args, no output), and SHA-256 of the output — never the
/// output itself. Useful for forensics after suspicious activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub audit_log: bool,
    pub audit_log_path: String,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            audit_log: false,
            audit_log_path: "~/.ntk/audit.log".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Root config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NtkConfig {
    pub daemon: DaemonConfig,
    pub compression: CompressionConfig,
    pub model: ModelConfig,
    pub metrics: MetricsConfig,
    pub exclusions: ExclusionsConfig,
    pub display: DisplayConfig,
    pub telemetry: TelemetryConfig,
    pub security: SecurityConfig,
    pub l3_cache: L3CacheConfig,
}

// ---------------------------------------------------------------------------
// Loading logic
// ---------------------------------------------------------------------------

/// Expand `~` at the start of a path string to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Load config from `~/.ntk/config.json`, falling back to defaults.
/// Then merge `.ntk.json` from `cwd` if it exists.
pub fn load(cwd: &Path) -> Result<NtkConfig> {
    let global_path = global_config_path()?;
    let mut config = load_file_or_default(&global_path)?;
    merge_local(&mut config, cwd)?;
    validate(&config)?;
    Ok(config)
}

/// Returns `~/.ntk/config.json`.
pub fn global_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".ntk").join("config.json"))
}

fn load_file_or_default(path: &Path) -> Result<NtkConfig> {
    if !path.exists() {
        return Ok(NtkConfig::default());
    }
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let config: NtkConfig =
        serde_json::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    Ok(config)
}

/// Merge fields present in `.ntk.json` (project-level) over the global config.
fn merge_local(base: &mut NtkConfig, cwd: &Path) -> Result<()> {
    let local_path = cwd.join(".ntk.json");
    if !local_path.exists() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(&local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;

    // Deserialize into a generic Value so we only override present fields.
    let local_val: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("parsing {}", local_path.display()))?;
    let base_val = serde_json::to_value(&*base).context("serializing base config")?;
    let merged = merge_json(base_val, local_val);
    *base = serde_json::from_value(merged).context("deserializing merged config")?;
    Ok(())
}

/// Recursively merge `b` into `a`: fields present in `b` override `a`.
fn merge_json(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    match (a, b) {
        (serde_json::Value::Object(mut a_map), serde_json::Value::Object(b_map)) => {
            for (k, v) in b_map {
                let entry = a_map.remove(&k).unwrap_or(serde_json::Value::Null);
                a_map.insert(k, merge_json(entry, v));
            }
            serde_json::Value::Object(a_map)
        }
        (_a, b) => b,
    }
}

// ---------------------------------------------------------------------------
// Validation (security gate)
// ---------------------------------------------------------------------------

fn validate(config: &NtkConfig) -> Result<()> {
    validate_ollama_url(&config.model.ollama_url)?;
    validate_bounds(config)?;
    Ok(())
}

/// Security: daemon must bind to a loopback address by default. Binding
/// to `0.0.0.0` or a public IP exposes every `Bash` tool output the hook
/// intercepts — including secrets echoed by commands like `env` or
/// `cat ~/.ssh/id_rsa` — to the local network.
///
/// Allowed by default: `127.0.0.1`, `::1`, `localhost`.
/// Escape hatch: set env `NTK_ALLOW_NON_LOOPBACK=1` (opt-in only).
pub fn is_loopback_host(host: &str) -> bool {
    use std::net::IpAddr;
    let trimmed = host.trim();
    if trimmed.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // Accept bracketed IPv6 notation `[::1]` as well as bare `::1`.
    let stripped = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    match stripped.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

/// Security: ollama_url must point to localhost (SSRF prevention).
fn validate_ollama_url(raw: &str) -> Result<()> {
    use url::Host;
    let url: url::Url = raw
        .parse()
        .with_context(|| format!("invalid ollama_url: {raw}"))?;
    let allowed = match url.host() {
        Some(Host::Domain(h)) => h == "localhost",
        Some(Host::Ipv4(addr)) => addr == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(addr)) => addr == std::net::Ipv6Addr::LOCALHOST,
        None => false,
    };
    if allowed {
        Ok(())
    } else {
        let host = url.host_str().unwrap_or("<none>");
        Err(anyhow!(
            "ollama_url must point to localhost (SSRF prevention), got: {host}"
        ))
    }
}

/// Validate numeric fields have sensible bounds.
fn validate_bounds(config: &NtkConfig) -> Result<()> {
    if config.exclusions.max_input_chars > 10_000_000 {
        return Err(anyhow!(
            "exclusions.max_input_chars too large (max 10_000_000)"
        ));
    }
    if config.compression.inference_threshold_tokens > 100_000 {
        return Err(anyhow!(
            "compression.inference_threshold_tokens too large (max 100_000)"
        ));
    }
    Ok(())
}

impl NtkConfig {
    pub fn storage_path_expanded(&self) -> PathBuf {
        expand_tilde(&self.metrics.storage_path)
    }
}

/// Resolve the system-prompts directory:
/// 1. `NTK_PROMPTS_DIR` env var
/// 2. `~/.ntk/system-prompts/`
/// 3. `./system-prompts/` (development fallback)
///
/// The `OllamaClient` falls back to embedded prompts if files are missing,
/// so any of these directories (even non-existent) are safe to pass.
pub fn resolve_prompts_dir() -> PathBuf {
    if let Ok(p) = std::env::var("NTK_PROMPTS_DIR") {
        return PathBuf::from(p);
    }
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".ntk").join("system-prompts");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("system-prompts")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn temp_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn test_load_default_when_file_missing() {
        let dir = temp_dir();
        // Point global config to a non-existent path by loading directly.
        let config = load_file_or_default(&dir.path().join("no_such_file.json")).unwrap();
        assert_eq!(config.daemon.port, 8765);
        assert_eq!(config.compression.inference_threshold_tokens, 300);
        assert!(config.compression.layer1_enabled);
    }

    #[test]
    fn test_merge_local_config_overrides_global() {
        let dir = temp_dir();
        // Write a minimal local override
        let local = serde_json::json!({
            "compression": { "inference_threshold_tokens": 150 },
            "model": { "model_name": "phi3:medium" }
        });
        fs::write(dir.path().join(".ntk.json"), local.to_string()).unwrap();

        let mut base = NtkConfig::default();
        merge_local(&mut base, dir.path()).unwrap();

        assert_eq!(base.compression.inference_threshold_tokens, 150);
        assert_eq!(base.model.model_name, "phi3:medium");
        // Untouched fields keep defaults
        assert_eq!(base.daemon.port, 8765);
    }

    #[test]
    fn test_expand_tilde_in_storage_path() {
        let config = NtkConfig::default();
        let expanded = config.storage_path_expanded();
        // Should not start with "~"
        assert!(!expanded.to_string_lossy().starts_with('~'));
        // Should end with metrics.db
        assert!(expanded.ends_with("metrics.db"));
    }

    #[test]
    fn test_validate_ollama_url_localhost_ok() {
        assert!(validate_ollama_url("http://localhost:11434").is_ok());
        assert!(validate_ollama_url("http://127.0.0.1:11434").is_ok());
        assert!(validate_ollama_url("http://[::1]:11434").is_ok());
    }

    #[test]
    fn test_validate_ollama_url_remote_rejected() {
        assert!(validate_ollama_url("http://192.168.1.100:11434").is_err());
        assert!(validate_ollama_url("http://ollama.internal:11434").is_err());
        assert!(validate_ollama_url("https://example.com/ollama").is_err());
    }

    #[test]
    fn test_is_loopback_host_accepts_loopback_variants() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(is_loopback_host("  127.0.0.1  "));
    }

    #[test]
    fn test_is_loopback_host_rejects_non_loopback() {
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("::"));
        assert!(!is_loopback_host("192.168.1.10"));
        assert!(!is_loopback_host("10.0.0.1"));
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host(""));
        assert!(!is_loopback_host("not-an-ip"));
    }

    #[test]
    fn test_load_full_config_from_file() {
        let dir = temp_dir();
        let json = serde_json::json!({
            "daemon": { "port": 9000, "log_level": "debug" },
            "compression": { "inference_threshold_tokens": 500 },
            "model": { "ollama_url": "http://127.0.0.1:11434" }
        });
        let path = dir.path().join("config.json");
        fs::write(&path, json.to_string()).unwrap();

        let config = load_file_or_default(&path).unwrap();
        assert_eq!(config.daemon.port, 9000);
        assert_eq!(config.daemon.log_level, "debug");
        assert_eq!(config.compression.inference_threshold_tokens, 500);
    }

    #[test]
    fn test_merge_does_not_affect_unspecified_fields() {
        let dir = temp_dir();
        let local = serde_json::json!({ "daemon": { "port": 9999 } });
        fs::write(dir.path().join(".ntk.json"), local.to_string()).unwrap();

        let mut base = NtkConfig::default();
        merge_local(&mut base, dir.path()).unwrap();

        assert_eq!(base.daemon.port, 9999);
        // host untouched
        assert_eq!(base.daemon.host, "127.0.0.1");
        // compression untouched
        assert_eq!(base.compression.inference_threshold_tokens, 300);
    }

    // --- Regression tests covering the three scenarios described in #14 ---

    #[test]
    fn test_merge_scenario_inherit_all() {
        // Empty local config → every global value is preserved.
        let dir = temp_dir();
        fs::write(dir.path().join(".ntk.json"), "{}").unwrap();

        let mut base = NtkConfig::default();
        let expected_port = base.daemon.port;
        let expected_host = base.daemon.host.clone();
        let expected_threshold = base.compression.inference_threshold_tokens;
        merge_local(&mut base, dir.path()).unwrap();

        assert_eq!(base.daemon.port, expected_port);
        assert_eq!(base.daemon.host, expected_host);
        assert_eq!(
            base.compression.inference_threshold_tokens,
            expected_threshold
        );
    }

    #[test]
    fn test_merge_scenario_partial_nested_override() {
        // Local overrides daemon.port only — daemon.host + other top-level
        // sections must remain at global defaults. Regression guard for the
        // bug described in issue #14.
        let dir = temp_dir();
        fs::write(
            dir.path().join(".ntk.json"),
            r#"{ "daemon": { "port": 7777 } }"#,
        )
        .unwrap();

        let mut base = NtkConfig::default();
        merge_local(&mut base, dir.path()).unwrap();

        assert_eq!(base.daemon.port, 7777);
        assert_eq!(base.daemon.host, "127.0.0.1", "host must NOT be zeroed");
        assert!(base.daemon.auto_start, "unspecified bool must NOT be false");
        assert_eq!(base.compression.inference_threshold_tokens, 300);
    }

    #[test]
    fn test_merge_scenario_total_override_of_a_section() {
        // Local specifies every field in the `daemon` section — those
        // override globally, and other sections stay intact.
        let dir = temp_dir();
        fs::write(
            dir.path().join(".ntk.json"),
            r#"{ "daemon": {
                "port": 1234,
                "host": "::1",
                "auto_start": false,
                "log_level": "debug"
            } }"#,
        )
        .unwrap();

        let mut base = NtkConfig::default();
        merge_local(&mut base, dir.path()).unwrap();

        assert_eq!(base.daemon.port, 1234);
        assert_eq!(base.daemon.host, "::1");
        assert!(!base.daemon.auto_start);
        assert_eq!(base.daemon.log_level, "debug");
        // Other sections untouched.
        assert_eq!(base.compression.inference_threshold_tokens, 300);
        assert!(base.compression.enabled);
    }

    #[test]
    fn test_merge_scenario_deep_nested_override() {
        // A sibling field inside the same deep object must not get zeroed
        // when only one sibling is overridden.
        let dir = temp_dir();
        fs::write(
            dir.path().join(".ntk.json"),
            r#"{ "compression": { "inference_threshold_tokens": 1000 } }"#,
        )
        .unwrap();

        let mut base = NtkConfig::default();
        merge_local(&mut base, dir.path()).unwrap();

        assert_eq!(base.compression.inference_threshold_tokens, 1000);
        assert!(
            base.compression.enabled,
            "sibling bool must stay at default"
        );
        assert!(
            base.compression.layer1_enabled,
            "deeper sibling bool must stay at default"
        );
    }
}
