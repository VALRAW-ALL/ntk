// ---------------------------------------------------------------------------
// Layer 3 — Local Inference via Ollama HTTP API
//
// Sends the L1+L2 compressed output to a local Ollama instance with a
// type-specific system prompt. Falls back gracefully (returns Err) on
// timeout or connection failure so the caller can use L1+L2 output instead.
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::time::Duration;

use crate::detector::OutputType;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Layer3Result {
    pub output: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
}

// ---------------------------------------------------------------------------
// OllamaClient
// ---------------------------------------------------------------------------

pub struct OllamaClient {
    base_url: String,
    timeout: Duration,
    model: String,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>, timeout_ms: u64, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout: Duration::from_millis(timeout_ms),
            model: model.into(),
        }
    }

    /// Compress `input` using the Ollama model with a type-specific system prompt.
    ///
    /// Returns `Err` on timeout or connection failure so the caller falls back to L1+L2.
    /// The content of `input` is placed in the *user* turn only — never in the system prompt
    /// (prevents prompt injection via crafted tool output).
    pub async fn compress(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> Result<Layer3Result> {
        let system_prompt = load_system_prompt(output_type, prompts_dir)?;

        let request_body = serde_json::json!({
            "model": self.model,
            "system": system_prompt,
            "prompt": input,
            "stream": false,
        });

        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .context("building reqwest client")?;

        let url = format!("{}/api/generate", self.base_url);
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| anyhow!("Ollama request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(anyhow!("Ollama returned HTTP {status}"));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("parsing Ollama response JSON")?;

        let compressed = body
            .get("response")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Ollama response missing 'response' field"))?
            .trim()
            .to_owned();

        if compressed.is_empty() {
            return Err(anyhow!("Ollama returned empty response"));
        }

        let input_tokens = body
            .get("prompt_eval_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let output_tokens = body.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        Ok(Layer3Result {
            output: compressed,
            input_tokens,
            output_tokens,
        })
    }
}

// ---------------------------------------------------------------------------
// System prompt loader
// ---------------------------------------------------------------------------

fn prompt_file_name(output_type: OutputType) -> &'static str {
    match output_type {
        OutputType::Test => "test.txt",
        OutputType::Build => "build.txt",
        OutputType::Log => "log.txt",
        OutputType::Diff => "diff.txt",
        OutputType::Generic => "log.txt", // closest fallback
    }
}

pub fn load_system_prompt(output_type: OutputType, prompts_dir: &Path) -> Result<String> {
    let file_name = prompt_file_name(output_type);
    let path = prompts_dir.join(file_name);
    if path.exists() {
        return std::fs::read_to_string(&path)
            .with_context(|| format!("loading system prompt from {}", path.display()));
    }
    // Embedded fallback prompts — binary works without prompt files installed.
    Ok(embedded_prompt(output_type).to_owned())
}

/// Embedded fallback system prompts (used when prompt files are not installed).
fn embedded_prompt(output_type: OutputType) -> &'static str {
    match output_type {
        OutputType::Test => {
            "You are a test output compressor. Produce EXACTLY this format:\n\
            1. X passed, Y failed, Z skipped\n\
            2. failing_test_name at file:line — expected: A, got: B\n\
            3. 1.23s\n\
            Rules: copy all numbers verbatim; copy expected/got in the SAME ORDER as the input, never swap them; \
            repeat line 2 once per failed test; omit line 3 if duration is absent; \
            no labels, no notes, no prose; stop after the last line."
        }
        OutputType::Build => {
            concat!(
                "You are a build output compressor. Extract ONLY: \
                (1) build result: success/failed; \
                (2) each ERROR: file:line + code + message (1 line); \
                (3) warning count only. No info messages, no progress.",
                " STRICT RULES: Output only data extracted from the input. \
                Do NOT add notes, assumptions, clarifications, or any sentence not found in the input. \
                If a value is absent, omit the field — do not guess or approximate it. \
                Stop immediately after the last extracted item."
            )
        }
        OutputType::Log | OutputType::Generic => {
            concat!(
                "You are a log compressor. Extract: \
                (1) all ERROR/CRITICAL lines with timestamps; \
                (2) WARN lines grouped as [xN] if repeated; \
                (3) first stack trace only; \
                (4) summary: X errors, Y warnings in N lines. Discard INFO/DEBUG.",
                " STRICT RULES: Output only data extracted from the input. \
                Do NOT add notes, assumptions, clarifications, or any sentence not found in the input. \
                If a value is absent, omit the field — do not guess or approximate it. \
                Stop immediately after the last extracted item."
            )
        }
        OutputType::Diff => {
            concat!(
                "You are a diff compressor. Extract: \
                (1) files changed; \
                (2) per-file: one-line summary of change; \
                (3) total: X files, +Y -Z lines. Discard unchanged context.",
                " STRICT RULES: Output only data extracted from the input. \
                Do NOT add notes, assumptions, clarifications, or any sentence not found in the input. \
                If a value is absent, omit the field — do not guess or approximate it. \
                Stop immediately after the last extracted item."
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Ollama management helpers (used by CLI commands)
// ---------------------------------------------------------------------------

/// Return the list of model names available in the local Ollama instance.
///
/// Calls `GET /api/tags` and extracts the `name` field of each model entry.
pub async fn list_models(base_url: &str, timeout_ms: u64) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .context("building reqwest client")?;

    let url = format!("{base_url}/api/tags");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow!("Ollama unreachable at {base_url}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(anyhow!("Ollama /api/tags returned HTTP {status}"));
    }

    let body: serde_json::Value = response.json().await.context("parsing Ollama tags JSON")?;
    let models = body
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    Ok(models)
}

/// Pull a model from Ollama via streaming NDJSON.
///
/// Prints progress to stdout as each layer downloads. Uses `\r` to overwrite
/// the current line so only the latest status is visible.
pub async fn pull_model(base_url: &str, model: &str, timeout_ms: u64) -> Result<()> {
    // Pull can take minutes — use a generous timeout based on caller's value.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .context("building reqwest client")?;

    let url = format!("{base_url}/api/pull");
    let body = serde_json::json!({ "name": model });

    let mut response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("Ollama unreachable at {base_url}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(anyhow!("Ollama /api/pull returned HTTP {status}"));
    }

    let mut last_status = String::new();
    let mut buf = String::new();

    // Use .chunk() to stream NDJSON without pulling the entire body into memory.
    while let Some(chunk) = response
        .chunk()
        .await
        .context("reading pull stream chunk")?
    {
        buf.push_str(std::str::from_utf8(&chunk).unwrap_or(""));

        // Process complete lines.
        while let Some(newline_pos) = buf.find('\n') {
            let line: String = buf.drain(..=newline_pos).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let status = val
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_owned();

                // Show layer-level progress: "downloading <digest> <completed>/<total>"
                if let (Some(completed), Some(total)) = (
                    val.get("completed").and_then(|v| v.as_u64()),
                    val.get("total").and_then(|v| v.as_u64()),
                ) {
                    let pct = if total > 0 {
                        completed
                            .saturating_mul(100)
                            .checked_div(total)
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    let mb_done = completed / 1_048_576;
                    let mb_total = total / 1_048_576;
                    print!("\r{status}  {mb_done}/{mb_total} MB ({pct}%)   ");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                } else if status != last_status {
                    if !last_status.is_empty() {
                        println!();
                    }
                    print!("{status}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    last_status = status;
                }
            }
        }
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_prompts(dir: &Path) {
        for name in &["test.txt", "build.txt", "log.txt", "diff.txt"] {
            std::fs::write(dir.join(name), format!("You are a {} compressor.", name)).unwrap();
        }
    }

    #[test]
    fn test_prompt_file_names_cover_all_types() {
        // Every OutputType variant must map to a non-empty filename.
        for t in [
            OutputType::Test,
            OutputType::Build,
            OutputType::Log,
            OutputType::Diff,
            OutputType::Generic,
        ] {
            let name = prompt_file_name(t);
            assert!(
                !name.is_empty(),
                "prompt_file_name returned empty for {t:?}"
            );
        }
    }

    #[test]
    fn test_load_system_prompt_returns_content() {
        let dir = TempDir::new().expect("tempdir");
        write_prompts(dir.path());

        let content = load_system_prompt(OutputType::Test, dir.path()).unwrap();
        assert!(
            content.contains("test.txt"),
            "expected prompt content: {content}"
        );
    }

    #[test]
    fn test_load_system_prompt_missing_file_uses_embedded_fallback() {
        let dir = TempDir::new().expect("tempdir");
        // Don't write any prompts — should fall back to embedded strings.
        let result = load_system_prompt(OutputType::Build, dir.path());
        assert!(
            result.is_ok(),
            "expected embedded fallback, got Err: {:?}",
            result.err()
        );
        let prompt = result.unwrap();
        assert!(
            !prompt.is_empty(),
            "embedded fallback prompt must not be empty"
        );
    }

    #[test]
    fn test_client_construction() {
        let client = OllamaClient::new("http://localhost:11434", 2000, "phi3:mini");
        assert_eq!(client.base_url, "http://localhost:11434");
        assert_eq!(client.model, "phi3:mini");
        assert_eq!(client.timeout, Duration::from_millis(2000));
    }
}
