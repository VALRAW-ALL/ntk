// ---------------------------------------------------------------------------
// Layer 3 — Candle In-Process Inference
//
// Loads a quantised GGUF model (Phi-3 Mini or any compatible model) directly
// inside the NTK process using HuggingFace Candle. No external daemon needed.
//
// Compile with:  cargo build --features candle
// With GPU:      cargo build --features cuda      (NVIDIA)
//                cargo build --features metal     (Apple Silicon)
//
// Required files in ~/.ntk/models/:
//   <model>.gguf      — quantised weights (2–3 GB, downloaded by `ntk model pull`)
//   tokenizer.json    — HuggingFace tokenizer spec (downloaded alongside model)
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

#[cfg(feature = "candle")]
use crate::compressor::layer3_inference::load_system_prompt;
use crate::compressor::layer3_inference::Layer3Result;
use crate::detector::OutputType;
#[cfg(feature = "candle")]
use anyhow::Context;

// ---------------------------------------------------------------------------
// Public struct — always compiled regardless of feature flag
// ---------------------------------------------------------------------------

pub struct CandleBackend {
    pub model_path: PathBuf,
    pub tokenizer_path: PathBuf,
    pub gpu: bool,
    pub max_new_tokens: usize,
    /// Lazily-initialised model state; only present when feature = "candle".
    #[cfg(feature = "candle")]
    state: std::sync::Arc<tokio::sync::Mutex<CandleState>>,
}

// ---------------------------------------------------------------------------
// Feature-gated internals
// ---------------------------------------------------------------------------

#[cfg(feature = "candle")]
enum CandleState {
    Unloaded,
    Loaded(Box<CandleLoaded>),
}

#[cfg(feature = "candle")]
struct CandleLoaded {
    model: candle_transformers::models::quantized_phi3::ModelWeights,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    eos_token: u32,
}

// ---------------------------------------------------------------------------
// Constructor — always compiled
// ---------------------------------------------------------------------------

impl CandleBackend {
    pub fn new(model_path: PathBuf, tokenizer_path: PathBuf, gpu: bool) -> Self {
        Self {
            model_path,
            tokenizer_path,
            gpu,
            max_new_tokens: 512,
            #[cfg(feature = "candle")]
            state: std::sync::Arc::new(tokio::sync::Mutex::new(CandleState::Unloaded)),
        }
    }

    // ---------------------------------------------------------------------------
    // compress — public API; returns Err immediately without `candle` feature
    // ---------------------------------------------------------------------------

    pub async fn compress(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> Result<Layer3Result> {
        #[cfg(not(feature = "candle"))]
        {
            let _ = (input, output_type, prompts_dir);
            Err(anyhow!(
                "Candle backend is not compiled.\n\
                Rebuild NTK with:  cargo build --release --features candle\n\
                GPU (NVIDIA):      cargo build --release --features cuda\n\
                GPU (Apple):       cargo build --release --features metal\n\
                Or switch backend: set model.provider = \"ollama\" in ~/.ntk/config.json"
            ))
        }

        #[cfg(feature = "candle")]
        self.compress_impl(input, output_type, prompts_dir).await
    }
}

// ---------------------------------------------------------------------------
// Feature-gated implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "candle")]
impl CandleBackend {
    async fn compress_impl(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> Result<Layer3Result> {
        use candle_core::{quantized::gguf_file, Device, Tensor};
        use candle_transformers::generation::LogitsProcessor;
        use candle_transformers::models::quantized_phi3;
        use tokenizers::Tokenizer;

        let system_prompt = load_system_prompt(output_type, prompts_dir)?;

        // ---- Lazy model loading (once per CandleBackend instance) ----
        let mut guard = self.state.lock().await;
        if matches!(*guard, CandleState::Unloaded) {
            tracing::info!("Candle: loading model from {}", self.model_path.display());

            let device = self.select_device()?;
            let model_path = self.model_path.clone();
            let tokenizer_path = self.tokenizer_path.clone();
            let device_clone = device.clone();

            // GGUF reading is blocking I/O — offload to a blocking thread.
            let loaded = tokio::task::spawn_blocking(move || -> Result<CandleLoaded> {
                let mut file = std::fs::File::open(&model_path)
                    .with_context(|| format!("opening {}", model_path.display()))?;

                let content = gguf_file::Content::read(&mut file)
                    .map_err(|e| anyhow!("reading GGUF file: {e}"))?;

                // Phi-3 Mini EOS token id from GGUF metadata; fall back to known default.
                let eos_token = content
                    .metadata
                    .get("tokenizer.ggml.eos_token_id")
                    .and_then(|v| {
                        if let gguf_file::Value::U32(n) = v {
                            Some(*n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(32007);

                // use_flash_attn=false: flash attention requires CUDA/Metal at runtime;
                // CPU inference always uses the standard attention path.
                let model =
                    quantized_phi3::ModelWeights::from_gguf(false, content, &mut file, &device_clone)
                        .map_err(|e| anyhow!("loading model weights: {e}"))?;

                let tokenizer = Tokenizer::from_file(&tokenizer_path)
                    .map_err(|e| anyhow!("loading tokenizer: {e}"))?;

                tracing::info!("Candle: model loaded (EOS token id = {eos_token})");
                Ok(CandleLoaded {
                    model,
                    tokenizer,
                    device: device_clone,
                    eos_token,
                })
            })
            .await
            .context("spawn_blocking model load")??;

            *guard = CandleState::Loaded(Box::new(loaded));
        }

        let CandleState::Loaded(ref mut loaded) = *guard else {
            return Err(anyhow!("model in unexpected state"));
        };

        // ---- Build Phi-3 chat-format prompt ----
        // <|system|>…<|end|>\n<|user|>…<|end|>\n<|assistant|>\n
        let prompt_text = format!(
            "<|system|>\n{system_prompt}<|end|>\n<|user|>\n{input}<|end|>\n<|assistant|>\n"
        );

        let encoding = loaded
            .tokenizer
            .encode(prompt_text.as_str(), true)
            .map_err(|e| anyhow!("encoding prompt: {e}"))?;

        let prompt_ids: Vec<u32> = encoding.get_ids().to_vec();
        let input_token_count = prompt_ids.len();

        // Prefill: feed the full prompt to build the KV cache.
        let prompt_tensor = Tensor::new(prompt_ids.as_slice(), &loaded.device)?.unsqueeze(0)?;
        loaded
            .model
            .forward(&prompt_tensor, 0)
            .map_err(|e| anyhow!("prefill pass: {e}"))?;

        // Decode: sample one token at a time.
        let mut last_token = *prompt_ids.last().unwrap_or(&0);
        let mut generated: Vec<u32> = Vec::with_capacity(self.max_new_tokens);
        let mut logits_processor = LogitsProcessor::new(42, Some(0.1_f64), None);

        for pos in input_token_count..input_token_count + self.max_new_tokens {
            let step_tensor = Tensor::new(&[last_token], &loaded.device)?.unsqueeze(0)?;
            let logits = loaded
                .model
                .forward(&step_tensor, pos)
                .map_err(|e| anyhow!("decode step {pos}: {e}"))?;

            let logits_1d = logits.squeeze(0)?.squeeze(0)?;
            let next = logits_processor
                .sample(&logits_1d)
                .map_err(|e| anyhow!("sampling at {pos}: {e}"))?;

            if next == loaded.eos_token {
                break;
            }
            generated.push(next);
            last_token = next;
        }

        let output_text = loaded
            .tokenizer
            .decode(&generated, true)
            .map_err(|e| anyhow!("decoding output: {e}"))?
            .trim()
            .to_owned();

        if output_text.is_empty() {
            return Err(anyhow!("Candle returned empty output"));
        }

        Ok(Layer3Result {
            output: output_text,
            input_tokens: input_token_count,
            output_tokens: generated.len(),
        })
    }

    fn select_device(&self) -> Result<candle_core::Device> {
        if !self.gpu {
            return Ok(candle_core::Device::Cpu);
        }
        #[cfg(feature = "cuda")]
        {
            match candle_core::Device::new_cuda(0) {
                Ok(d) => {
                    tracing::info!("Candle: using CUDA device 0");
                    return Ok(d);
                }
                Err(e) => tracing::warn!("Candle: CUDA unavailable ({e}), falling back to CPU"),
            }
        }
        #[cfg(all(feature = "metal", not(feature = "cuda")))]
        {
            match candle_core::Device::new_metal(0) {
                Ok(d) => {
                    tracing::info!("Candle: using Metal device 0");
                    return Ok(d);
                }
                Err(e) => tracing::warn!("Candle: Metal unavailable ({e}), falling back to CPU"),
            }
        }
        Ok(candle_core::Device::Cpu)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Default GGUF model path under ~/.ntk/models/.
pub fn default_model_path(quant: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".ntk").join("models").join(format!(
        "Phi-3-mini-4k-instruct-{}.gguf",
        quant.to_uppercase()
    )))
}

/// Default tokenizer path under ~/.ntk/models/.
pub fn default_tokenizer_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".ntk").join("models").join("tokenizer.json"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_candle_backend_construction() {
        let b = CandleBackend::new(
            PathBuf::from("/tmp/model.gguf"),
            PathBuf::from("/tmp/tokenizer.json"),
            false,
        );
        assert_eq!(b.model_path, PathBuf::from("/tmp/model.gguf"));
        assert!(!b.gpu);
        assert_eq!(b.max_new_tokens, 512);
    }

    #[tokio::test]
    async fn test_candle_compress_fails_without_model_file() {
        let dir = TempDir::new().unwrap();
        let b = CandleBackend::new(
            dir.path().join("nonexistent.gguf"),
            dir.path().join("tokenizer.json"),
            false,
        );
        let result = b.compress("test", OutputType::Test, dir.path()).await;
        assert!(result.is_err());
        // With feature: fails because file doesn't exist.
        // Without feature: fails with "not compiled" message.
    }

    #[test]
    fn test_default_model_path_contains_gguf() {
        let p = default_model_path("q5_k_m").unwrap();
        assert!(p.to_str().unwrap().contains(".gguf"));
        assert!(p.to_str().unwrap().contains("q5_k_m") || p.to_str().unwrap().contains("Q5_K_M"));
    }

    #[test]
    fn test_default_tokenizer_path() {
        let p = default_tokenizer_path().unwrap();
        assert!(p.to_str().unwrap().ends_with("tokenizer.json"));
    }
}
