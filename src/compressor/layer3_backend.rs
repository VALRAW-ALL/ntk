// ---------------------------------------------------------------------------
// Layer 3 Backend Dispatcher
//
// `BackendKind` is an exhaustive enum that selects between the three Layer 3
// inference backends at runtime, based on `config.model.provider`.
//
//   Ollama   — external HTTP daemon   (default, zero compile-time cost)
//   Candle   — in-process GGUF        (compile with --features candle)
//   LlamaCpp — llama-server subprocess (always compiled, no extra deps)
//
// The dispatch is an `enum` match instead of `dyn trait + async_trait` to
// avoid the object-safety complexity of async methods.
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::Arc;

use crate::compressor::layer3_candle::CandleBackend;
use crate::compressor::layer3_inference::{Layer3Result, OllamaClient};
use crate::compressor::layer3_llamacpp::LlamaCppBackend;
use crate::config::{ModelProvider, NtkConfig};
use crate::detector::OutputType;

// ---------------------------------------------------------------------------
// BackendKind
// ---------------------------------------------------------------------------

pub enum BackendKind {
    Ollama(OllamaClient),
    Candle(CandleBackend),
    LlamaCpp(Arc<LlamaCppBackend>),
}

impl std::fmt::Debug for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BackendKind::{}", self.name())
    }
}

impl BackendKind {
    // ---------------------------------------------------------------------------
    // Factory — builds the appropriate backend from config
    // ---------------------------------------------------------------------------

    pub fn from_config(config: &NtkConfig) -> Result<Self> {
        match config.model.provider {
            ModelProvider::Ollama => {
                let client = OllamaClient::new(
                    &config.model.ollama_url,
                    config.model.timeout_ms,
                    &config.model.model_name,
                );
                Ok(Self::Ollama(client))
            }

            ModelProvider::Candle => {
                use crate::compressor::layer3_candle::{
                    default_model_path, default_tokenizer_path,
                };

                let model_path = config.model.model_path.clone().unwrap_or_else(|| {
                    default_model_path(&config.model.quantization).unwrap_or_default()
                });

                let tokenizer_path = config
                    .model
                    .tokenizer_path
                    .clone()
                    .unwrap_or_else(|| default_tokenizer_path().unwrap_or_default());

                let gpu = config.model.gpu_auto_detect || config.model.gpu_layers != 0;
                let backend = CandleBackend::new(model_path, tokenizer_path, gpu);
                Ok(Self::Candle(backend))
            }

            ModelProvider::LlamaCpp => {
                let model_path = config
                    .model
                    .model_path
                    .clone()
                    .ok_or_else(|| {
                        anyhow!(
                            "model.model_path is required for llama.cpp backend.\n\
                            Set it in ~/.ntk/config.json:  \"model_path\": \"~/.ntk/models/phi3.gguf\""
                        )
                    })?;

                let port = config.model.llama_server_port;
                let gpu_layers = config.model.gpu_layers;
                let backend =
                    LlamaCppBackend::new(model_path, port, gpu_layers, config.model.timeout_ms)
                        .with_start_timeout(config.model.llama_server_start_timeout_ms)
                        .with_gpu_selection(config.model.gpu_vendor, config.model.cuda_device);

                Ok(Self::LlamaCpp(Arc::new(backend)))
            }
        }
    }

    // ---------------------------------------------------------------------------
    // compress — dispatches to the selected backend
    // ---------------------------------------------------------------------------

    pub async fn compress(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> Result<Layer3Result> {
        match self {
            Self::Ollama(client) => client.compress(input, output_type, prompts_dir).await,
            Self::Candle(backend) => backend.compress(input, output_type, prompts_dir).await,
            Self::LlamaCpp(backend) => backend.compress(input, output_type, prompts_dir).await,
        }
    }

    /// Start the underlying subprocess if applicable (llama.cpp only).
    /// No-op for Ollama and Candle.
    pub async fn start_if_needed(&self) -> Result<()> {
        if let Self::LlamaCpp(backend) = self {
            backend.start().await?;
        }
        Ok(())
    }

    /// Human-readable backend name (for status and logging).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ollama(_) => "ollama",
            Self::Candle(_) => "candle",
            Self::LlamaCpp(_) => "llama.cpp",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> NtkConfig {
        NtkConfig::default()
    }

    #[test]
    fn test_from_config_ollama_default() {
        let config = default_config();
        let backend = BackendKind::from_config(&config).unwrap();
        assert_eq!(backend.name(), "ollama");
    }

    #[test]
    fn test_from_config_candle() {
        let mut config = default_config();
        config.model.provider = ModelProvider::Candle;
        // model_path is None → falls back to default path (no file check at creation).
        let backend = BackendKind::from_config(&config).unwrap();
        assert_eq!(backend.name(), "candle");
    }

    #[test]
    fn test_from_config_llamacpp_without_model_path_returns_error() {
        let mut config = default_config();
        config.model.provider = ModelProvider::LlamaCpp;
        config.model.model_path = None;
        config.model.llama_server_auto_start = false;
        let result = BackendKind::from_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("model_path"));
    }

    #[test]
    fn test_from_config_llamacpp_with_model_path_no_autostart() {
        let mut config = default_config();
        config.model.provider = ModelProvider::LlamaCpp;
        config.model.model_path = Some(std::path::PathBuf::from("/tmp/model.gguf"));
        config.model.llama_server_auto_start = false; // don't start subprocess in test
        let backend = BackendKind::from_config(&config).unwrap();
        assert_eq!(backend.name(), "llama.cpp");
    }
}
