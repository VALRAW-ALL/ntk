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
// BackendChain — ordered fallback across multiple backends
// ---------------------------------------------------------------------------

/// Wraps an ordered list of backends. On `compress`, tries each in order
/// and returns the first success. On all-fail, propagates the last error
/// and lets the caller decide whether to degrade to L1+L2.
///
/// A single-element chain is indistinguishable from `BackendKind` alone,
/// which keeps the no-config-change migration path open.
pub struct BackendChain {
    backends: Vec<Arc<BackendKind>>,
}

impl std::fmt::Debug for BackendChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendChain")
            .field("backends", &self.names())
            .finish()
    }
}

impl BackendChain {
    /// Build a single-element chain from an already-constructed
    /// `BackendKind`. Used by the daemon's safety net when config
    /// parsing fails but we still want a usable chain.
    pub fn from_single(backend: BackendKind) -> Self {
        Self {
            backends: vec![Arc::new(backend)],
        }
    }

    /// Build a chain from `config.model.backend_chain`, or fall back to a
    /// single-element chain from `config.model.provider` when the chain
    /// is empty. Unknown names in the chain are skipped with a warning;
    /// an all-empty result returns an error so the daemon never starts
    /// with zero backends.
    pub fn from_config(config: &NtkConfig) -> Result<Self> {
        let chain = &config.model.backend_chain;
        if chain.is_empty() {
            let backend = BackendKind::from_config(config)?;
            return Ok(Self {
                backends: vec![Arc::new(backend)],
            });
        }

        let mut backends: Vec<Arc<BackendKind>> = Vec::with_capacity(chain.len());
        for name in chain {
            let mut spec = config.clone();
            spec.model.provider = match name.as_str() {
                "ollama" => ModelProvider::Ollama,
                "candle" => ModelProvider::Candle,
                "llama.cpp" | "llamacpp" | "llama_cpp" => ModelProvider::LlamaCpp,
                other => {
                    tracing::warn!("backend_chain: skipping unknown backend '{other}'");
                    continue;
                }
            };
            match BackendKind::from_config(&spec) {
                Ok(b) => backends.push(Arc::new(b)),
                Err(e) => tracing::warn!("backend_chain: skipping '{name}': {e}"),
            }
        }
        if backends.is_empty() {
            return Err(anyhow!(
                "backend_chain produced zero usable backends — check config.model.backend_chain"
            ));
        }
        Ok(Self { backends })
    }

    /// Name of the first (primary) backend. Exposed for status and
    /// metrics; downstream code that needs to know the currently-active
    /// backend per request should consult the future BackendUsed signal
    /// emitted after compress().
    pub fn name(&self) -> &'static str {
        // Safe: from_config guarantees at least one element.
        self.backends.first().map(|b| b.name()).unwrap_or("(none)")
    }

    /// All backend names in order — used by `ntk status` to report the
    /// configured chain.
    pub fn names(&self) -> Vec<&'static str> {
        self.backends.iter().map(|b| b.name()).collect()
    }

    /// Try each backend in order. Returns the first success plus the
    /// name of the backend that produced it. All-fail returns the last
    /// error wrapped with the backend list so the caller can surface
    /// which chain failed.
    pub async fn compress(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> Result<(Layer3Result, &'static str)> {
        let mut last_err: Option<anyhow::Error> = None;
        for backend in &self.backends {
            match backend.compress(input, output_type, prompts_dir).await {
                Ok(result) => return Ok((result, backend.name())),
                Err(e) => {
                    tracing::warn!(
                        "backend '{name}' failed, trying next: {e}",
                        name = backend.name()
                    );
                    last_err = Some(e);
                }
            }
        }
        let chain: Vec<&'static str> = self.backends.iter().map(|b| b.name()).collect();
        Err(last_err.unwrap_or_else(|| anyhow!("no backends configured (chain: {chain:?})")))
    }

    /// Start each backend that needs a subprocess (currently only
    /// llama.cpp). Errors on any single backend do not abort the chain —
    /// the backend that fails to start is skipped at compress time via
    /// the normal fallback path.
    pub async fn start_if_needed(&self) {
        for backend in &self.backends {
            if let Err(e) = backend.start_if_needed().await {
                tracing::warn!("backend '{name}' start failed: {e}", name = backend.name());
            }
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

    #[test]
    fn test_backend_chain_empty_config_falls_back_to_provider() {
        let config = default_config();
        assert!(config.model.backend_chain.is_empty());
        let chain = BackendChain::from_config(&config).unwrap();
        assert_eq!(chain.name(), "ollama");
        assert_eq!(chain.names(), vec!["ollama"]);
    }

    #[test]
    fn test_backend_chain_explicit_chain_is_respected() {
        let mut config = default_config();
        // candle + ollama — candle builds fine even without a real model
        // file present (no IO at construction), so this exercises the
        // multi-backend construction path.
        config.model.backend_chain = vec!["candle".to_string(), "ollama".to_string()];
        let chain = BackendChain::from_config(&config).unwrap();
        assert_eq!(chain.names(), vec!["candle", "ollama"]);
        assert_eq!(chain.name(), "candle", "primary is first element");
    }

    #[test]
    fn test_backend_chain_unknown_names_are_skipped() {
        let mut config = default_config();
        config.model.backend_chain = vec![
            "nonesuch".to_string(),
            "ollama".to_string(),
            "alsofake".to_string(),
        ];
        let chain = BackendChain::from_config(&config).unwrap();
        assert_eq!(chain.names(), vec!["ollama"]);
    }

    #[test]
    fn test_backend_chain_all_invalid_returns_error() {
        let mut config = default_config();
        config.model.backend_chain = vec!["nope".to_string(), "alsonope".to_string()];
        let err = BackendChain::from_config(&config).unwrap_err();
        assert!(err.to_string().contains("zero usable backends"));
    }
}
