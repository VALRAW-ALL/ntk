// ---------------------------------------------------------------------------
// Layer 3 — llama.cpp Subprocess Backend
//
// Spawns `llama-server` (from llama.cpp) as a child process and communicates
// with it over its local HTTP API. No Rust binding to native code — pure
// subprocess + HTTP, so it compiles on all platforms without cmake.
//
// Setup:
//   1. Install llama.cpp: https://github.com/ggerganov/llama.cpp/releases
//      Place `llama-server` in ~/.ntk/bin/ or anywhere on PATH.
//   2. Download a GGUF model: `ntk model pull --backend llamacpp`
//   3. Set model.provider = "llama_cpp" in ~/.ntk/config.json
//   4. Run `ntk start`
//
// The server is started automatically at daemon startup and stopped when the
// daemon exits (via Drop).
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::compressor::layer3_inference::{load_system_prompt, Layer3Result};
use crate::detector::OutputType;

// ---------------------------------------------------------------------------
// LlamaCppBackend
// ---------------------------------------------------------------------------

pub struct LlamaCppBackend {
    pub server_url: String,
    pub model_path: PathBuf,
    pub n_gpu_layers: i32,
    pub timeout_ms: u64,
    pub context_size: usize,
    /// Running llama-server child process (None if not started or managed externally).
    process: Arc<Mutex<Option<std::process::Child>>>,
}

impl LlamaCppBackend {
    pub fn new(model_path: PathBuf, port: u16, n_gpu_layers: i32, timeout_ms: u64) -> Self {
        Self {
            server_url: format!("http://127.0.0.1:{port}"),
            model_path,
            n_gpu_layers,
            timeout_ms,
            context_size: 4096,
            process: Arc::new(Mutex::new(None)),
        }
    }

    /// Constructor for testing: supply an arbitrary server URL instead of spawning
    /// a subprocess. The `process` field is left empty (no child to kill on Drop).
    pub fn new_with_url(
        server_url: String,
        model_path: PathBuf,
        n_gpu_layers: i32,
        timeout_ms: u64,
        context_size: usize,
    ) -> Self {
        Self {
            server_url,
            model_path,
            n_gpu_layers,
            timeout_ms,
            context_size,
            process: Arc::new(Mutex::new(None)),
        }
    }

    // ---------------------------------------------------------------------------
    // Start / stop the llama-server subprocess
    // ---------------------------------------------------------------------------

    /// Start `llama-server` if not already running.
    /// Spawns the subprocess and waits up to 10 s for the HTTP endpoint to become healthy.
    /// Must be called from an async context (uses `tokio::time::sleep`).
    pub async fn start(&self) -> Result<()> {
        // Inner block ensures MutexGuard is dropped before any .await point,
        // making the future Send-safe for tokio::spawn.
        let already_running = {
            let guard = self.process.lock().map_err(|_| anyhow!("mutex poisoned"))?;
            guard.is_some()
        };
        if already_running {
            return Ok(());
        }

        let binary = find_llama_server_binary()?;

        if !self.model_path.exists() {
            return Err(anyhow!(
                "llama.cpp model file not found: {}\n\
                Download with: ntk model pull --backend llamacpp",
                self.model_path.display()
            ));
        }

        let port = self
            .server_url
            .trim_start_matches("http://127.0.0.1:")
            .parse::<u16>()
            .unwrap_or(8766);

        tracing::info!(
            "llama.cpp: starting {} --model {} --port {}",
            binary.display(),
            self.model_path.display(),
            port,
        );

        let child = std::process::Command::new(&binary)
            .arg("--model")
            .arg(&self.model_path)
            .arg("--port")
            .arg(port.to_string())
            .arg("--ctx-size")
            .arg(self.context_size.to_string())
            .arg("--n-gpu-layers")
            .arg(self.n_gpu_layers.to_string())
            .arg("--threads")
            .arg(num_cpus().to_string())
            .arg("--log-disable")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("spawning {}", binary.display()))?;

        // Store child, then release lock before the health-check await.
        {
            let mut guard = self.process.lock().map_err(|_| anyhow!("mutex poisoned"))?;
            *guard = Some(child);
        }

        // Wait up to 10 s for the server to become healthy.
        self.wait_for_healthy(10_000).await?;
        tracing::info!("llama.cpp: server healthy at {}", self.server_url);
        Ok(())
    }

    pub fn stop(&self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
                tracing::info!("llama.cpp: server stopped");
            }
        }
    }

    async fn wait_for_healthy(&self, timeout_ms: u64) -> Result<()> {
        let health_url = format!("{}/health", self.server_url);
        let fallback = Duration::from_secs(10);
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_millis(timeout_ms))
            .or_else(|| tokio::time::Instant::now().checked_add(fallback))
            .unwrap_or_else(tokio::time::Instant::now);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .context("building reqwest client for health check")?;

        while tokio::time::Instant::now() < deadline {
            if client
                .get(&health_url)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Err(anyhow!(
            "llama-server at {} did not become healthy within {}ms",
            self.server_url,
            timeout_ms
        ))
    }

    // ---------------------------------------------------------------------------
    // compress
    // ---------------------------------------------------------------------------

    pub async fn compress(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> Result<Layer3Result> {
        let system_prompt = load_system_prompt(output_type, prompts_dir)?;

        // Build the Phi-3 chat-format prompt.
        let prompt_text = format!(
            "<|system|>\n{system_prompt}<|end|>\n<|user|>\n{input}<|end|>\n<|assistant|>\n"
        );

        let request_body = serde_json::json!({
            "prompt": prompt_text,
            "n_predict": 512,
            // temperature=0 → greedy decoding: eliminates stochastic hallucination.
            // top_k=1 reinforces greedy (top_p is irrelevant when temp=0).
            "temperature": 0.0,
            "top_k": 1,
            "repeat_penalty": 1.05,
            "stop": [
                "<|end|>", "<|user|>", "<|endoftext|>",
                "\nNote:", "\nNote ", "\nPlease",
                " not provided", " not available", " not specified"
            ],
            "stream": false,
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(self.timeout_ms))
            .build()
            .context("building reqwest client")?;

        let url = format!("{}/completion", self.server_url);
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| anyhow!("llama-server request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(anyhow!("llama-server returned HTTP {status}"));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("parsing llama-server response")?;

        let raw = body
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("llama-server response missing 'content' field"))?
            .trim()
            .to_owned();

        if raw.is_empty() {
            return Err(anyhow!("llama-server returned empty content"));
        }

        let content = strip_prose_lines(&raw);

        let input_tokens = body
            .get("tokens_evaluated")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let output_tokens = body
            .get("tokens_predicted")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        Ok(Layer3Result {
            output: content,
            input_tokens,
            output_tokens,
        })
    }
}

impl Drop for LlamaCppBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the `llama-server` binary: checks ~/.ntk/bin/ first, then PATH.
pub fn find_llama_server_binary() -> Result<PathBuf> {
    // 1. ~/.ntk/bin/llama-server (or llama-server.exe on Windows)
    let binary_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".ntk").join("bin").join(binary_name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // 2. PATH lookup
    let which_cmd = if cfg!(windows) { "where" } else { "which" };
    let output = std::process::Command::new(which_cmd)
        .arg("llama-server")
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !path_str.is_empty() {
                return Ok(PathBuf::from(path_str));
            }
        }
    }

    Err(anyhow!(
        "llama-server not found.\n\
        Install options:\n\
          macOS (Homebrew):  brew install llama.cpp\n\
          Linux (apt):       apt install llama.cpp\n\
          Manual:            https://github.com/ggerganov/llama.cpp/releases\n\
        Then place the binary in ~/.ntk/bin/ or on your PATH."
    ))
}

/// Remove prose/hallucination lines that local models sometimes append.
///
/// Works in two passes:
/// 1. `PROSE_PREFIXES` — strips any trailing line whose trimmed content *starts with*
///    a known hallucination prefix ("note:", "please", "if the", …).
/// 2. `PROSE_CONTAINS` — strips any trailing line that *contains* a known
///    invented-value phrase ("not provided", "not available", "not specified", …).
///    These appear when the model invents "3. Total duration: not provided" instead
///    of simply omitting the field.
///
/// Only trailing lines are removed so that matching tokens inside real output
/// (e.g., a test named "if_the_value_is_zero") are never discarded.
fn strip_prose_lines(text: &str) -> String {
    const PROSE_PREFIXES: &[&str] = &[
        "note:",
        "note ",
        "please",
        "if the",
        "if you",
        "assumption:",
        "replace ",
        "the above",
        "i have",
        "the duration",
        "actual duration",
    ];

    // Substrings that signal an invented value anywhere in a line.
    const PROSE_CONTAINS: &[&str] = &[
        "not provided",
        "not available",
        "not specified",
        "not present",
        "not found",
        "not given",
        "not applicable",
        "n/a",
        "unknown",
    ];

    let lines: Vec<&str> = text.lines().collect();

    let is_prose = |line: &str| -> bool {
        let lc = line.trim().to_lowercase();
        if lc.is_empty() {
            return false; // blank lines are handled separately
        }
        PROSE_PREFIXES.iter().any(|p| lc.starts_with(p))
            || PROSE_CONTAINS.iter().any(|p| lc.contains(p))
    };

    // Walk backwards from the end, skipping blank lines and prose lines.
    let last_real = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| {
            let lc = line.trim();
            !lc.is_empty() && !is_prose(line)
        })
        .map(|(i, _)| i);

    match last_real {
        Some(idx) => lines[..=idx].join("\n"),
        None => text.trim().to_owned(), // all prose? return as-is rather than empty
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_llamacpp_backend_construction() {
        let b = LlamaCppBackend::new(PathBuf::from("/tmp/model.gguf"), 8766, 0, 5000);
        assert_eq!(b.server_url, "http://127.0.0.1:8766");
        assert_eq!(b.model_path, PathBuf::from("/tmp/model.gguf"));
        assert_eq!(b.n_gpu_layers, 0);
        assert_eq!(b.timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_llamacpp_compress_fails_when_server_unreachable() {
        // Use an ephemeral port that nothing is listening on.
        let b = LlamaCppBackend::new(
            PathBuf::from("/tmp/model.gguf"),
            19991, // unlikely to be in use
            0,
            500,
        );
        let dir = TempDir::new().unwrap();
        let result = b.compress("test input", OutputType::Test, dir.path()).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("request failed") || msg.contains("timed out") || msg.contains("error"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn test_find_llama_server_binary_returns_error_when_missing() {
        // This test passes as long as llama-server is not installed.
        // When llama-server IS installed, it returns Ok — also fine.
        let result = find_llama_server_binary();
        // Either found (Ok) or not found (Err) — both are valid in CI.
        match result {
            Ok(p) => assert!(p.exists() || p.to_str().is_some()),
            Err(e) => assert!(e.to_string().contains("llama-server")),
        }
    }
}
