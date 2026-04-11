use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "ntk", version, about = "Neural Token Killer — semantic compression for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Install NTK and register the PostToolUse hook in your editor settings.
    Init {
        /// Install globally (modifies ~/.claude/settings.json or ~/.opencode/settings.json).
        #[arg(short, long)]
        global: bool,

        /// Target OpenCode instead of Claude Code.
        #[arg(long)]
        opencode: bool,

        /// Patch settings.json without prompting.
        #[arg(long)]
        auto_patch: bool,

        /// Only install the hook script — skip config.json creation.
        #[arg(long)]
        hook_only: bool,

        /// Show current installation status (read-only).
        #[arg(long)]
        show: bool,

        /// Remove the NTK hook from editor settings.
        #[arg(long)]
        uninstall: bool,
    },

    /// Start the NTK daemon.
    Start {
        /// Enable GPU inference (requires Candle CUDA/Metal or Ollama GPU).
        #[arg(long)]
        gpu: bool,
    },

    /// Stop the running NTK daemon.
    Stop,

    /// Show daemon status, loaded model, and GPU info.
    Status,

    /// Show per-command token savings metrics table.
    Metrics,

    /// Print an ASCII bar chart of token savings over time.
    Graph,

    /// Show token savings summary (RTK-compatible format).
    Gain,

    /// Show compression history log.
    History,

    /// Show or edit the active configuration.
    Config {
        /// Path to a custom config file.
        #[arg(long)]
        file: Option<PathBuf>,
    },

    /// Test compression on a captured output file.
    #[command(name = "test-compress")]
    TestCompress {
        /// Path to the file to compress.
        file: PathBuf,
    },

    /// Manage the local inference model.
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },

    /// Analyze missed NTK/RTK opportunities in the current session.
    Discover,

    /// Run correctness tests on all compression layers with built-in payloads.
    Test {
        /// Also test Layer 3 inference (requires Ollama running).
        #[arg(long)]
        l3: bool,
    },

    /// Benchmark all compression layers with built-in payloads.
    Bench {
        /// Number of runs per payload (default: 5).
        #[arg(long, default_value = "5")]
        runs: usize,

        /// Also benchmark Layer 3 inference (requires Ollama running).
        #[arg(long)]
        l3: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ModelAction {
    /// Interactive wizard: compare backends and configure one.
    Setup,
    /// Download the inference model (model and/or tokenizer).
    Pull {
        /// Quantization variant, e.g. q4_k_m, q5_k_m, q6_k (default: q5_k_m).
        #[arg(long, default_value = "q5_k_m")]
        quant: String,
        /// Which backend to pull for: ollama | candle | llamacpp.
        #[arg(long, default_value = "ollama")]
        backend: String,
    },
    /// Test model latency and output quality.
    Test,
    /// Benchmark CPU vs GPU inference speed.
    Bench,
    /// List available models in the configured backend.
    List,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // Default (no subcommand): start daemon.
        None => run_daemon(false),

        Some(Command::Init { global, opencode, auto_patch, hook_only, show, uninstall }) => {
            run_init(global, opencode, auto_patch, hook_only, show, uninstall)
        }

        Some(Command::Start { gpu }) => run_daemon(gpu),

        Some(Command::Stop) => run_stop(),

        Some(Command::Status) => run_status(),

        Some(Command::Metrics) => run_metrics(),

        Some(Command::Graph) => run_graph(),

        Some(Command::Gain) => run_gain(),

        Some(Command::History) => run_history(),

        Some(Command::Config { file }) => run_config(file),

        Some(Command::TestCompress { file }) => run_test_compress(&file),

        Some(Command::Model { action }) => run_model(action),


        Some(Command::Discover) => run_discover(),

        Some(Command::Test { l3 }) => run_test(l3),

        Some(Command::Bench { runs, l3 }) => run_bench(runs, l3),
    }
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn run_init(
    _global: bool,
    opencode: bool,
    auto_patch: bool,
    hook_only: bool,
    show: bool,
    uninstall: bool,
) -> Result<()> {
    use ntk::installer::{EditorTarget, Installer};

    let editor = if opencode {
        EditorTarget::OpenCode
    } else {
        EditorTarget::ClaudeCode
    };

    let installer = Installer {
        editor,
        auto_patch,
        hook_only,
    };

    if show {
        installer.show_status()
    } else if uninstall {
        installer.uninstall()
    } else {
        installer.run()
    }
}

fn run_daemon(gpu: bool) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_run_daemon(gpu))
}

async fn async_run_daemon(gpu: bool) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd)?;

    if gpu {
        tracing::info!("GPU inference requested — auto-detecting backend");
    }

    let host = config.daemon.host.clone();
    let port = config.daemon.port;
    let addr = format!("{host}:{port}");

    // Initialize SQLite db if metrics are enabled.
    let db = if config.metrics.enabled {
        let db_path = config.storage_path_expanded();
        match ntk::metrics::MetricsDb::init(&db_path).await {
            Ok(d) => {
                tracing::info!("SQLite metrics db initialized at {}", db_path.display());
                Some(Arc::new(d))
            }
            Err(e) => {
                tracing::warn!("SQLite metrics init failed (in-memory only): {e}");
                None
            }
        }
    } else {
        None
    };

    // Build Layer 3 backend.
    let backend = match ntk::compressor::layer3_backend::BackendKind::from_config(&config) {
        Ok(b) => {
            tracing::info!("Layer 3 backend: {}", b.name());
            Arc::new(b)
        }
        Err(e) => {
            tracing::warn!("Layer 3 backend init failed, falling back to Ollama: {e}");
            Arc::new(
                ntk::compressor::layer3_backend::BackendKind::from_config(
                    &ntk::config::NtkConfig::default(),
                )
                .expect("default Ollama backend must always succeed"),
            )
        }
    };

    // Start subprocess if backend requires it (llama.cpp auto_start).
    // Runs in background so the daemon accepts connections immediately.
    if config.model.llama_server_auto_start {
        let backend_bg = Arc::clone(&backend);
        tokio::spawn(async move {
            if let Err(e) = backend_bg.start_if_needed().await {
                tracing::warn!("llama-server auto-start failed: {e}");
            } else {
                tracing::info!("llama-server ready");
            }
        });
    }

    let state = ntk::server::AppState {
        config: Arc::new(config),
        metrics: Arc::new(Mutex::new(ntk::metrics::MetricsStore::new())),
        db,
        backend,
    };

    let router = ntk::server::build_router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("NTK daemon listening on {addr}");
    println!("NTK daemon started on {addr}  (Ctrl-C to stop)");
    axum::serve(listener, router).await?;
    Ok(())
}

fn run_stop() -> Result<()> {
    // Read PID file and send SIGTERM (Unix) / TerminateProcess (Windows).
    let pid_path = pid_file_path()?;
    if !pid_path.exists() {
        println!("NTK daemon is not running (no PID file found).");
        return Ok(());
    }
    let pid_str = std::fs::read_to_string(&pid_path)?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid PID file"))?;

    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
            .map_err(|e| anyhow!("failed to stop daemon (PID {pid}): {e}"))?;
        println!("Sent SIGTERM to NTK daemon (PID {pid}).");
    }

    #[cfg(windows)]
    {
        // Safety: using WinAPI to terminate a process by PID we own.
        unsafe {
            use windows_sys::Win32::System::Threading::{
                OpenProcess, TerminateProcess, PROCESS_TERMINATE,
            };
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle == 0 {
                return Err(anyhow!("cannot open process (PID {pid}) — already stopped?"));
            }
            TerminateProcess(handle, 0);
        }
        println!("Terminated NTK daemon (PID {pid}).");
    }

    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

fn run_status() -> Result<()> {
    use ntk::gpu;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();

    // Check daemon health.
    let url = daemon_url()?;
    let (daemon_ok, daemon_info) = match ureq_get(&format!("{url}/health")) {
        Ok(body) => {
            let val = serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default();
            let uptime = val["uptime_secs"].as_u64().unwrap_or(0);
            (true, format!("running  (uptime {uptime}s)"))
        }
        Err(_) => (false, "stopped  (run `ntk start`)".to_owned()),
    };

    // Check Ollama + list available models.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (ollama_ok, model_list) = rt.block_on(async {
        match ntk::compressor::layer3_inference::list_models(
            &config.model.ollama_url,
            config.model.timeout_ms,
        )
        .await
        {
            Ok(models) => (true, models),
            Err(_) => (false, vec![]),
        }
    });

    let backend = gpu::detect_best_backend();

    use ntk::output::terminal as term;

    let ok = term::ok_mark();
    let err = term::err_mark();

    term::print_header("NTK Status", "══════════════════════════════════════════");

    let daemon_icon = if daemon_ok { &ok } else { &err };
    println!("  Daemon       : {daemon_icon} {daemon_info}");
    println!("  Endpoint     : {}{url}{}", term::dim(), term::reset());
    println!("  GPU backend  : {}{backend}{}", term::cyan(), term::reset());
    println!("  Config       : {}{}{}",
        term::dim(),
        ntk::config::global_config_path().map(|p| p.display().to_string()).unwrap_or_else(|_| "unknown".to_owned()),
        term::reset()
    );
    println!();

    let provider_name = match config.model.provider {
        ntk::config::ModelProvider::Ollama => "ollama",
        ntk::config::ModelProvider::Candle => "candle",
        ntk::config::ModelProvider::LlamaCpp => "llama.cpp",
    };
    println!("{}{}Model ({}):{}",
        term::bold(), term::bright_cyan(), provider_name, term::reset());
    println!("  Configured : {}{}{}", term::cyan(), config.model.model_name, term::reset());
    println!("  Quantize   : {}", config.model.quantization);
    if ollama_ok {
        println!("  Ollama     : {} reachable  ({}{}{})",
            ok, term::dim(), config.model.ollama_url, term::reset());
        if model_list.is_empty() {
            println!("  Available  : {}(none — run `ntk model pull`){}", term::yellow(), term::reset());
        } else {
            println!("  Available  :");
            for m in &model_list {
                let is_active = m.contains(&config.model.model_name)
                    || config.model.model_name.contains(m.split(':').next().unwrap_or(""));
                if is_active {
                    println!("    {} {}{m}{} {}◀ active{}",
                        ok, term::bright_green(), term::reset(), term::dim(), term::reset());
                } else {
                    println!("    {} {}{m}{}", term::dim(), term::reset(), term::reset());
                }
            }
        }
    } else if daemon_ok {
        println!("  Ollama     : {} unreachable — L3 falls back to L1+L2", err);
    } else {
        println!("  Ollama     : {}(daemon not running){}", term::dim(), term::reset());
    }
    println!();

    let on_off = |v: bool| -> String {
        if v { format!("{}{}on{}", term::bold(), term::bright_green(), term::reset()) }
        else  { format!("{}off{}", term::dim(), term::reset()) }
    };

    println!("{}{}Compression:{}",
        term::bold(), term::bright_cyan(), term::reset());
    println!("  L1 filter   : {}", on_off(config.compression.layer1_enabled));
    println!("  L2 tokenize : {}", on_off(config.compression.layer2_enabled));
    println!("  L3 infer    : {}", on_off(config.compression.layer3_enabled));
    println!("  L3 threshold: {}{} tokens{}",
        term::cyan(), config.compression.inference_threshold_tokens, term::reset());
    Ok(())
}

fn run_metrics() -> Result<()> {
    let url = daemon_url()?;
    let response = ureq_get(&format!("{url}/metrics"))?;
    println!("{response}");
    Ok(())
}

fn run_graph() -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();
    let db_path = config.storage_path_expanded();

    if !db_path.exists() {
        println!("[ntk graph] No metrics database found. Start the daemon and run some commands first.");
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let db = ntk::metrics::MetricsDb::init(&db_path).await?;
        let rows = db.history(100).await?;

        // Convert HistoryRow back to CompressionRecord-like data for the chart.
        let records: Vec<ntk::metrics::CompressionRecord> = rows
            .iter()
            .map(|r| ntk::metrics::CompressionRecord {
                command: r.command.clone(),
                output_type: ntk::detector::OutputType::Generic,
                original_tokens: r.original_tokens,
                compressed_tokens: r.compressed_tokens,
                layer_used: r.layer_used,
                latency_ms: r.latency_ms,
                rtk_pre_filtered: false,
                timestamp: chrono::Utc::now(),
            })
            .collect();

        ntk::output::graph::print_bar_chart(&records);
        let summary = ntk::metrics::MetricsDb::init(&db_path)
            .await?
            .summary(config.metrics.history_days)
            .await?;
        println!();
        println!(
            "Total: {} tokens saved across {} compressions ({:.0}% avg ratio)",
            summary.total_tokens_saved,
            summary.total_compressions,
            summary.average_ratio * 100.0
        );
        Ok(())
    })
}

fn run_gain() -> Result<()> {
    let url = daemon_url()?;
    let response = ureq_get(&format!("{url}/metrics"))?;
    // Parse and reformat as RTK-compatible gain output.
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&response) {
        let saved = val["total_tokens_saved"].as_u64().unwrap_or(0);
        let calls = val["total_calls"].as_u64().unwrap_or(0);
        let pct = val["average_ratio"].as_f64().unwrap_or(0.0) * 100.0;
        println!("NTK: {saved} tokens saved across {calls} compressions ({pct:.0}% avg)");
    } else {
        println!("{response}");
    }
    Ok(())
}

fn run_history() -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();
    let db_path = config.storage_path_expanded();

    if !db_path.exists() {
        println!("No history database found at {}.", db_path.display());
        println!("Start the daemon and run some compressions first.");
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let db = ntk::metrics::MetricsDb::init(&db_path).await?;
        let rows = db.history(50).await?;

        if rows.is_empty() {
            println!("No compression history yet.");
            return Ok(());
        }

        println!(
            "{:<22}  {:<8}  {:>8}  {:>8}  {:>6}  {:>5}  TIME",
            "COMMAND", "TYPE", "BEFORE", "AFTER", "RATIO", "LAYER",
        );
        println!("{}", "-".repeat(80));

        for r in &rows {
            let cmd = if r.command.len() > 22 {
                &r.command[..22]
            } else {
                &r.command
            };
            let saved = r.original_tokens.saturating_sub(r.compressed_tokens);
            let ratio_pct = if r.original_tokens > 0 {
                (saved as f64 / r.original_tokens as f64 * 100.0).round() as u64
            } else {
                0
            };
            println!(
                "{:<22}  {:<8}  {:>8}  {:>8}  {:>5}%  L{:<4}  {}",
                cmd,
                r.output_type,
                r.original_tokens,
                r.compressed_tokens,
                ratio_pct,
                r.layer_used,
                r.created_at,
            );
        }
        Ok(())
    })
}

fn run_config(file: Option<PathBuf>) -> Result<()> {
    let path = if let Some(f) = file {
        f
    } else {
        ntk::config::global_config_path()?
    };
    if !path.exists() {
        println!("Config file not found: {}", path.display());
        println!("Run `ntk init` to create a default config.");
        return Ok(());
    }
    let contents = std::fs::read_to_string(&path)?;
    println!("{contents}");
    Ok(())
}

fn run_test_compress(file: &std::path::Path) -> Result<()> {
    use ntk::compressor::{layer1_filter, layer2_tokenizer};

    let canonical = file
        .canonicalize()
        .map_err(|e| anyhow!("cannot read {}: {e}", file.display()))?;

    let input = std::fs::read_to_string(&canonical)
        .map_err(|e| anyhow!("reading {}: {e}", canonical.display()))?;

    let original_tokens = layer2_tokenizer::count_tokens(&input)?;
    let l1 = layer1_filter::filter(&input);
    let l2 = layer2_tokenizer::process(&l1.output)?;

    let ratio = if original_tokens > 0 {
        let saved = original_tokens.saturating_sub(l2.compressed_tokens);
        (saved as f64 / original_tokens as f64) * 100.0
    } else {
        0.0
    };

    println!("File:             {}", canonical.display());
    println!("Original tokens:  {original_tokens}");
    println!("L1 lines removed: {}", l1.lines_removed);
    println!("After L2 tokens:  {}", l2.compressed_tokens);
    println!("Compression:      {ratio:.1}%");
    println!();
    println!("--- Compressed output ---");
    println!("{}", l2.output);
    Ok(())
}

fn run_model(action: ModelAction) -> Result<()> {
    match action {
        ModelAction::Setup => run_model_setup(),
        ModelAction::Pull { quant, backend } => run_model_pull(&quant, &backend),
        ModelAction::Test => run_model_test(),
        ModelAction::Bench => run_model_bench(),
        ModelAction::List => run_model_list(),
    }
}

fn run_model_pull(quant: &str, backend: &str) -> Result<()> {
    match backend {
        "ollama" => run_model_pull_ollama(),
        "candle" | "llamacpp" | "llama.cpp" => run_model_pull_gguf(quant, backend),
        other => {
            println!("Unknown backend '{other}'. Use: ollama | candle | llamacpp");
            std::process::exit(1);
        }
    }
}

fn run_model_pull_ollama() -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();

    // Ollama uses its own model names (phi3:mini), not GGUF naming (phi3:q5_k_m).
    let model = &config.model.model_name;
    println!("Pulling {model} via Ollama ({})…", config.model.ollama_url);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(ntk::compressor::layer3_inference::pull_model(
        &config.model.ollama_url,
        model,
        10 * 60 * 1000, // 10-minute timeout for large models
    ))?;
    println!("Done. Run `ntk model test` to verify inference.");
    Ok(())
}

fn run_model_pull_gguf(quant: &str, backend: &str) -> Result<()> {
    use ntk::compressor::layer3_candle::{default_model_path, default_tokenizer_path};

    let model_path = default_model_path(quant)?;
    let tokenizer_path = default_tokenizer_path()?;

    // Create the models directory if needed.
    if let Some(parent) = model_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    println!("Backend  : {backend}");
    println!("Quant    : {quant}");
    println!("Model →  {}", model_path.display());
    println!("Tokenizer→ {}", tokenizer_path.display());
    println!();

    // ---- Download GGUF model ----
    if model_path.exists() {
        println!("Model file already present — skipping download.");
        println!("Delete it to force a fresh download: rm \"{}\"", model_path.display());
    } else {
        // Phi-3 Mini GGUF files from Bartowski's HuggingFace mirror (all quants available).
        let quant_upper = quant.to_uppercase();
        let gguf_url = format!(
            "https://huggingface.co/bartowski/Phi-3-mini-4k-instruct-GGUF/resolve/main/Phi-3-mini-4k-instruct-{quant_upper}.gguf"
        );
        println!("Downloading model ({quant_upper})…");
        download_file_with_progress(&gguf_url, &model_path)?;
        println!("\nModel saved to {}", model_path.display());
    }

    // ---- Download tokenizer ----
    if tokenizer_path.exists() {
        println!("Tokenizer already present — skipping.");
    } else {
        let tok_url =
            "https://huggingface.co/microsoft/Phi-3-mini-4k-instruct/resolve/main/tokenizer.json";
        println!("Downloading tokenizer…");
        download_file_with_progress(tok_url, &tokenizer_path)?;
        println!("\nTokenizer saved to {}", tokenizer_path.display());
    }

    println!();
    println!("Add these lines to ~/.ntk/config.json to activate:");
    println!("  \"model\": {{");
    println!("    \"provider\": \"{backend}\",");
    println!("    \"model_path\": \"{}\",", model_path.display());
    if backend == "candle" {
        println!("    \"tokenizer_path\": \"{}\",", tokenizer_path.display());
    }
    println!("    \"quantization\": \"{quant}\"");
    println!("  }}");
    println!();
    println!("Or run `ntk model setup` for an interactive wizard.");
    Ok(())
}

fn run_model_list() -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();

    let provider_name = match config.model.provider {
        ntk::config::ModelProvider::Ollama => "ollama",
        ntk::config::ModelProvider::Candle => "candle",
        ntk::config::ModelProvider::LlamaCpp => "llama.cpp",
    };
    println!("Backend: {provider_name}");

    if matches!(config.model.provider, ntk::config::ModelProvider::Ollama) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let models = rt
            .block_on(ntk::compressor::layer3_inference::list_models(
                &config.model.ollama_url,
                config.model.timeout_ms,
            ))
            .unwrap_or_default();
        if models.is_empty() {
            println!("No models installed. Run: ntk model pull");
        } else {
            println!("Installed models:");
            for m in &models {
                let active = if m.contains(&config.model.model_name) { " ← active" } else { "" };
                println!("  {m}{active}");
            }
        }
    } else {
        // Candle / llama.cpp: just show the configured GGUF path.
        if let Some(p) = &config.model.model_path {
            let status = if p.exists() { "✓ found" } else { "✗ not found" };
            println!("Model path: {} [{status}]", p.display());
        } else {
            let default = ntk::compressor::layer3_candle::default_model_path(&config.model.quantization)
                .unwrap_or_default();
            let status = if default.exists() { "✓ found" } else { "✗ not found" };
            println!("Model path: {} [{status}] (default)", default.display());
            println!("Set model.model_path in ~/.ntk/config.json to use a custom path.");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ntk model setup — interactive backend wizard
// ---------------------------------------------------------------------------

fn run_model_setup() -> Result<()> {
    use std::io::{self, BufRead, Write};
    use ntk::gpu;
    use ntk::output::terminal as term;

    println!();
    println!("{}{}  NTK Model Setup Wizard  {}", term::bold(), term::bright_cyan(), term::reset());
    println!("{}══════════════════════════════════════════════════════════════════{}", term::dim(), term::reset());
    println!();

    // ---- System detection (with spinner) ----
    let sp = term::Spinner::start("Detecting system…");

    let gpu_backend = gpu::detect_best_backend();
    let gpu_str = gpu_backend.to_string();
    let has_gpu = !gpu_str.contains("CPU") && !gpu_str.contains("Scalar");

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let ollama_ok = rt
        .block_on(ntk::compressor::layer3_inference::list_models(
            &config.model.ollama_url,
            1500,
        ))
        .is_ok();

    let llamacpp_ok = ntk::compressor::layer3_llamacpp::find_llama_server_binary().is_ok();
    let candle_compiled = cfg!(feature = "candle");

    sp.finish();

    // ---- System info ----
    println!("{}{}System Info{}", term::bold(), term::bright_cyan(), term::reset());
    println!("{}  ────────────────────────────────────{}", term::dim(), term::reset());

    let gpu_color = if has_gpu { term::bright_green() } else { term::yellow() };
    let gpu_icon = if has_gpu { "✓" } else { "◌" };
    println!("  {}GPU / CPU{}    {}{} {}{}", term::dim(), term::reset(), gpu_color, gpu_icon, gpu_str, term::reset());

    let ollama_color = if ollama_ok { term::bright_green() } else { term::yellow() };
    let ollama_icon = if ollama_ok { "✓" } else { "◌" };
    let ollama_status = if ollama_ok { "running" } else { "not running" };
    println!("  {}Ollama{}       {}{} {}{}", term::dim(), term::reset(), ollama_color, ollama_icon, ollama_status, term::reset());

    let llama_color = if llamacpp_ok { term::bright_green() } else { term::yellow() };
    let llama_icon = if llamacpp_ok { "✓" } else { "◌" };
    let llama_status = if llamacpp_ok { "found in PATH / ~/.ntk/bin" } else { "not found" };
    println!("  {}llama-server{} {}{} {}{}", term::dim(), term::reset(), llama_color, llama_icon, llama_status, term::reset());

    let candle_color = if candle_compiled { term::bright_green() } else { term::yellow() };
    let candle_icon = if candle_compiled { "✓" } else { "◌" };
    let candle_status = if candle_compiled { "compiled in" } else { "needs --features candle" };
    println!("  {}Candle{}       {}{} {}{}", term::dim(), term::reset(), candle_color, candle_icon, candle_status, term::reset());
    println!();

    // ---- Comparison table (plain text inside cells to keep alignment) ----
    println!("{}{}Backend Comparison{}", term::bold(), term::bright_cyan(), term::reset());
    println!("  ┌──────────────────┬─────────────────┬──────────────────────────────────────┐");
    println!("  │ {}Backend{}          │ {}Availability{}    │ {}Summary{}                              │",
        term::bold(), term::reset(), term::bold(), term::reset(), term::bold(), term::reset());
    println!("  ├──────────────────┼─────────────────┼──────────────────────────────────────┤");

    let ollama_av = if ollama_ok { "✓ running       " } else { "◌ needs install  " };
    println!("  │ {}[1] Ollama{}       │ {} │ External daemon, any model, easiest  │",
        term::bold(), term::reset(), ollama_av);

    let candle_av = if candle_compiled { "✓ compiled      " } else { "◌ needs rebuild  " };
    println!("  │ {}[2] Candle{}       │ {} │ In-process GGUF, no daemon           │",
        term::bold(), term::reset(), candle_av);

    let llama_av = if llamacpp_ok { "✓ found         " } else { "◌ needs install  " };
    println!("  │ {}[3] llama.cpp{}    │ {} │ Subprocess, best CPU performance     │",
        term::bold(), term::reset(), llama_av);

    println!("  └──────────────────┴─────────────────┴──────────────────────────────────────┘");
    println!();

    // ---- Pros / cons ----
    println!("{}{}Pros & Cons{}", term::bold(), term::bright_cyan(), term::reset());
    println!();

    println!("  {}[1] Ollama{}", term::bold(), term::reset());
    println!("    {}+{} Easiest setup — just `ollama pull phi3:mini`", term::bright_green(), term::reset());
    println!("    {}+{} Supports hundreds of models (llama3, mistral, gemma2, qwen2…)", term::bright_green(), term::reset());
    println!("    {}+{} Auto GPU/CPU fallback, model management built-in", term::bright_green(), term::reset());
    println!("    {}−{} Requires Ollama daemon running alongside NTK (two processes)", term::bright_red(), term::reset());
    println!("    {}−{} External installation: {}https://ollama.ai{}", term::bright_red(), term::reset(), term::dim(), term::reset());
    println!();

    println!("  {}[2] Candle{}  {}(in-process — requires: cargo build --features candle){}", term::bold(), term::reset(), term::dim(), term::reset());
    println!("    {}+{} Single binary, no external processes", term::bright_green(), term::reset());
    println!("    {}+{} Direct CUDA/Metal/CPU access, lowest overhead", term::bright_green(), term::reset());
    println!("    {}+{} Works offline once GGUF + tokenizer.json downloaded", term::bright_green(), term::reset());
    println!("    {}−{} Requires recompiling NTK with --features candle", term::bright_red(), term::reset());
    println!("    {}−{} GGUF + tokenizer.json must be downloaded (~2.2 GB)", term::bright_red(), term::reset());
    println!("    {}−{} Limited to GGUF-format models", term::bright_red(), term::reset());
    println!();

    println!("  {}[3] llama.cpp{}", term::bold(), term::reset());
    println!("    {}+{} Best CPU performance (AVX2 / AMX optimisations)", term::bright_green(), term::reset());
    println!("    {}+{} Excellent CUDA/Metal GPU support", term::bright_green(), term::reset());
    println!("    {}+{} Works offline once GGUF downloaded (~2.2 GB)", term::bright_green(), term::reset());
    println!("    {}−{} Requires llama-server: {}brew install llama.cpp  or GitHub releases{}", term::bright_red(), term::reset(), term::dim(), term::reset());
    println!("    {}−{} Extra process to manage (auto-started by NTK daemon)", term::bright_red(), term::reset());
    println!();

    // ---- Recommendation ----
    let recommended: u8 = if ollama_ok {
        1
    } else if llamacpp_ok {
        3
    } else if candle_compiled {
        2
    } else {
        1
    };

    let rec_name = match recommended { 2 => "Candle", 3 => "llama.cpp", _ => "Ollama" };
    println!("  {}🎯  Recommendation: [{}] {}{}", term::bright_yellow(), recommended, rec_name, term::reset());
    if recommended == 1 && !ollama_ok {
        println!("  {}    Install Ollama at https://ollama.ai then run: ollama serve{}", term::dim(), term::reset());
    }
    println!();

    // ---- User choice ----
    print!("{}Choose backend [1/2/3] or Enter for [{}]:{} ", term::bold(), recommended, term::reset());
    io::stdout().flush()?;

    let stdin = io::stdin();
    let choice_str = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
    let choice = choice_str.trim().parse::<u8>().unwrap_or(recommended);

    println!();
    match choice {
        1 => setup_write_config("ollama", &config)?,
        2 => setup_candle(&config)?,
        3 => setup_llamacpp(&config)?,
        _ => {
            println!("{}✗  Invalid choice. Run `ntk model setup` again.{}", term::bright_red(), term::reset());
            return Ok(());
        }
    }

    println!();
    println!("{}✓{}  Configuration saved.  {}Restart NTK daemon:{} ntk stop && ntk start",
        term::bright_green(), term::reset(), term::bold(), term::reset());
    Ok(())
}

fn setup_write_config(provider: &str, existing: &ntk::config::NtkConfig) -> Result<()> {
    use ntk::output::terminal as term;

    let sp = term::Spinner::start("Saving configuration…");

    let global_path = ntk::config::global_config_path()?;
    let mut config = existing.clone();

    if provider == "ollama" {
        config.model.provider = ntk::config::ModelProvider::Ollama;
    }

    // Write atomically.
    let json = serde_json::to_string_pretty(&config)?;
    let tmp = global_path.with_extension("tmp");
    if let Some(parent) = global_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &global_path)?;

    sp.finish_ok(&format!(
        "{}~/.ntk/config.json{}  {}provider = {}{}",
        term::bold(), term::reset(), term::dim(), provider, term::reset()
    ));

    println!();
    println!("  {}Next steps for Ollama:{}", term::bold(), term::reset());
    println!("  {}1.{} Install Ollama:     {}https://ollama.ai{}", term::bright_cyan(), term::reset(), term::dim(), term::reset());
    println!("  {}2.{} Pull the model:     {}ollama pull phi3:mini{}", term::bright_cyan(), term::reset(), term::dim(), term::reset());
    println!("  {}3.{} Start the daemon:   {}ntk start{}", term::bright_cyan(), term::reset(), term::dim(), term::reset());
    Ok(())
}

fn setup_candle(existing: &ntk::config::NtkConfig) -> Result<()> {
    use std::io::{self, BufRead, Write};
    use ntk::output::terminal as term;

    if !cfg!(feature = "candle") {
        println!("{}{}Candle is not compiled in the current binary.{}", term::bold(), term::bright_yellow(), term::reset());
        println!();
        println!("  Rebuild NTK with the Candle feature flag:");
        println!("  {}Standard (CPU){}  cargo build --release --features candle", term::dim(), term::reset());
        println!("  {}NVIDIA GPU{}     cargo build --release --features cuda", term::dim(), term::reset());
        println!("  {}Apple GPU{}      cargo build --release --features metal", term::dim(), term::reset());
        println!();
        println!("  {}Then run: ntk model setup{}", term::dim(), term::reset());
        return Ok(());
    }

    println!("{}{}[2] Candle — In-process inference{}", term::bold(), term::bright_cyan(), term::reset());
    println!("{}  ────────────────────────────────────{}", term::dim(), term::reset());

    let quant = &existing.model.quantization;
    let model_path = ntk::compressor::layer3_candle::default_model_path(quant)?;
    let tokenizer_path = ntk::compressor::layer3_candle::default_tokenizer_path()?;

    let need_model = !model_path.exists();
    let need_tokenizer = !tokenizer_path.exists();

    let model_icon = if need_model { format!("{}◌ missing{}", term::yellow(), term::reset()) } else { format!("{}✓ found{}", term::bright_green(), term::reset()) };
    let tok_icon   = if need_tokenizer { format!("{}◌ missing{}", term::yellow(), term::reset()) } else { format!("{}✓ found{}", term::bright_green(), term::reset()) };

    println!("  {}Model{}      {}  {}", term::dim(), term::reset(), model_path.display(), model_icon);
    println!("  {}Tokenizer{} {}  {}", term::dim(), term::reset(), tokenizer_path.display(), tok_icon);
    println!();

    if need_model || need_tokenizer {
        let missing_label = match (need_model, need_tokenizer) {
            (true, true)  => "model + tokenizer (~2.2 GB)",
            (true, false) => "model (~2.2 GB)",
            _             => "tokenizer (~1 MB)",
        };
        print!("{}Download missing files now?{}  {}[{}]  {}{}", term::bold(), term::reset(), term::dim(), missing_label, term::reset(),
               format!("{}[Y/n]:{} ", term::bright_cyan(), term::reset()));
        io::stdout().flush()?;
        let answer = io::stdin()
            .lock()
            .lines()
            .next()
            .unwrap_or(Ok(String::new()))?;
        let answer = answer.trim().to_lowercase();
        if answer != "n" && answer != "no" {
            println!();
            if let Some(parent) = model_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if need_tokenizer {
                let tok_url = "https://huggingface.co/microsoft/Phi-3-mini-4k-instruct/resolve/main/tokenizer.json";
                println!("  {}Downloading tokenizer…{}", term::dim(), term::reset());
                download_file_with_progress(tok_url, &tokenizer_path)?;
                println!("\r  {}✓{}  tokenizer.json                                    ", term::bright_green(), term::reset());
            }
            if need_model {
                let quant_upper = quant.to_uppercase();
                let gguf_url = format!(
                    "https://huggingface.co/bartowski/Phi-3-mini-4k-instruct-GGUF/resolve/main/Phi-3-mini-4k-instruct-{quant_upper}.gguf"
                );
                println!("  {}Downloading model  ({quant_upper}, ~2.2 GB)…{}", term::dim(), term::reset());
                download_file_with_progress(&gguf_url, &model_path)?;
                println!("\r  {}✓{}  Phi-3-mini-4k-instruct-{quant_upper}.gguf                ", term::bright_green(), term::reset());
            }
            println!();
        }
    } else {
        println!("  {}✓  All files already present — no download needed.{}", term::bright_green(), term::reset());
        println!();
    }

    let mut config = existing.clone();
    config.model.provider = ntk::config::ModelProvider::Candle;
    config.model.model_path = Some(model_path);
    config.model.tokenizer_path = Some(tokenizer_path);

    let sp = term::Spinner::start("Saving configuration…");
    let global_path = ntk::config::global_config_path()?;
    let json = serde_json::to_string_pretty(&config)?;
    let tmp = global_path.with_extension("tmp");
    if let Some(parent) = global_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &global_path)?;
    sp.finish_ok(&format!(
        "{}~/.ntk/config.json{}  {}provider = candle{}",
        term::bold(), term::reset(), term::dim(), term::reset()
    ));
    Ok(())
}

fn setup_llamacpp(existing: &ntk::config::NtkConfig) -> Result<()> {
    use std::io::{self, BufRead, Write};
    use ntk::output::terminal as term;

    println!("{}{}[3] llama.cpp — Subprocess inference{}", term::bold(), term::bright_cyan(), term::reset());
    println!("{}  ────────────────────────────────────{}", term::dim(), term::reset());
    println!();

    // Check for llama-server — offer auto-install if missing or incomplete.
    let server_binary = match ntk::compressor::layer3_llamacpp::find_llama_server_binary() {
        Ok(p) => {
            // Verify companion shared libraries are present next to the binary.
            let bin_dir = p.parent().unwrap_or(std::path::Path::new("."));
            let missing_libs: Vec<&str> = core_lib_stems()
                .iter()
                .copied()
                .filter(|&lib| !bin_dir.join(lib).exists())
                .collect();

            if !missing_libs.is_empty() {
                let lib_ext = if cfg!(windows) { "DLLs" }
                    else if cfg!(target_os = "macos") { "dylibs" }
                    else { "shared libs (.so)" };
                println!("  {}llama-server{}  {}{}{}  {}companion {} missing: {}{}",
                    term::dim(), term::reset(),
                    term::yellow(), p.display(), term::reset(),
                    term::bright_red(), lib_ext, missing_libs.join(", "), term::reset());
                println!();
                print!("  {}Re-download llama-server with all shared libraries?{}  {}[Y/n]:{} ",
                    term::bold(), term::reset(), term::bright_cyan(), term::reset());
                io::stdout().flush()?;
                let answer = io::stdin()
                    .lock()
                    .lines()
                    .next()
                    .unwrap_or(Ok(String::new()))?;
                if answer.trim().to_lowercase() == "n" {
                    println!("  {}◌  Keeping existing binary — server may fail to start.{}", term::yellow(), term::reset());
                    p
                } else {
                    println!();
                    install_llama_server_binary()?
                }
            } else {
                println!("  {}llama-server{}  {}✓ {}{}",
                    term::dim(), term::reset(), term::bright_green(), p.display(), term::reset());
                p
            }
        }
        Err(_) => {
            println!("  {}llama-server{}  {}◌ not found in PATH or ~/.ntk/bin{}", term::dim(), term::reset(), term::yellow(), term::reset());
            println!();
            print!("  {}Download and install llama-server automatically?{}  {}[Y/n]:{} ",
                term::bold(), term::reset(), term::bright_cyan(), term::reset());
            io::stdout().flush()?;
            let answer = io::stdin()
                .lock()
                .lines()
                .next()
                .unwrap_or(Ok(String::new()))?;
            if answer.trim().to_lowercase() == "n" {
                println!();
                println!("  {}Manual install options:{}", term::bold(), term::reset());
                println!("  {}macOS (Homebrew){}  brew install llama.cpp", term::dim(), term::reset());
                println!("  {}Linux (apt){}      apt install llama.cpp", term::dim(), term::reset());
                println!("  {}Releases page{}    {}https://github.com/ggerganov/llama.cpp/releases{}", term::dim(), term::reset(), term::dim(), term::reset());
                println!();
                println!("  Place llama-server in {}~/.ntk/bin/{} or on your PATH, then run `ntk model setup` again.", term::bold(), term::reset());
                return Ok(());
            }
            println!();
            install_llama_server_binary()?
        }
    };
    println!("  {}✓{}  llama-server ready: {}", term::bright_green(), term::reset(), server_binary.display());
    println!();

    let quant = &existing.model.quantization;
    let model_path = ntk::compressor::layer3_candle::default_model_path(quant)?;

    let model_icon = if model_path.exists() { format!("{}✓ found{}", term::bright_green(), term::reset()) } else { format!("{}◌ missing{}", term::yellow(), term::reset()) };
    println!("  {}Model{}  {}  {}", term::dim(), term::reset(), model_path.display(), model_icon);
    println!();

    if !model_path.exists() {
        let quant_upper = quant.to_uppercase();
        print!("  {}Download GGUF model now?{}  {}({quant_upper}, ~2.2 GB){}  {}[Y/n]:{} ",
            term::bold(), term::reset(), term::dim(), term::reset(), term::bright_cyan(), term::reset());
        io::stdout().flush()?;
        let answer = io::stdin()
            .lock()
            .lines()
            .next()
            .unwrap_or(Ok(String::new()))?;
        if answer.trim().to_lowercase() != "n" {
            println!();
            if let Some(parent) = model_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let gguf_url = format!(
                "https://huggingface.co/bartowski/Phi-3-mini-4k-instruct-GGUF/resolve/main/Phi-3-mini-4k-instruct-{quant_upper}.gguf"
            );
            println!("  {}Downloading model  ({quant_upper}, ~2.2 GB)…{}", term::dim(), term::reset());
            download_file_with_progress(&gguf_url, &model_path)?;
            println!("\r  {}✓{}  Phi-3-mini-4k-instruct-{quant_upper}.gguf                ", term::bright_green(), term::reset());
            println!();
        }
    } else {
        println!("  {}✓  Model already present — no download needed.{}", term::bright_green(), term::reset());
        println!();
    }

    let mut config = existing.clone();
    config.model.provider = ntk::config::ModelProvider::LlamaCpp;
    config.model.model_path = Some(model_path);

    let sp = term::Spinner::start("Saving configuration…");
    let global_path = ntk::config::global_config_path()?;
    let json = serde_json::to_string_pretty(&config)?;
    let tmp = global_path.with_extension("tmp");
    if let Some(parent) = global_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &global_path)?;
    sp.finish_ok(&format!(
        "{}~/.ntk/config.json{}  {}provider = llama_cpp{}",
        term::bold(), term::reset(), term::dim(), term::reset()
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// llama-server automatic installation
// ---------------------------------------------------------------------------

/// Download the latest llama-server binary from GitHub Releases, extract it,
/// and place it in `~/.ntk/bin/`. Returns the installed path on success.
fn install_llama_server_binary() -> Result<PathBuf> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(install_llama_server_binary_async())
}

async fn install_llama_server_binary_async() -> Result<PathBuf> {
    use ntk::output::terminal as term;

    // Destination dir: ~/.ntk/bin/
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    let bin_dir = home.join(".ntk").join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    let binary_name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let dest = bin_dir.join(binary_name);

    // ---- 1. Fetch latest release info from GitHub API ----
    let sp = term::Spinner::start("Fetching latest llama.cpp release from GitHub…");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("ntk-installer/0.1")
        .build()?;

    let release: serde_json::Value = client
        .get("https://api.github.com/repos/ggerganov/llama.cpp/releases/latest")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("GitHub API request failed: {e}"))?
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("parsing GitHub release JSON: {e}"))?;

    let tag = release.get("tag_name").and_then(|v| v.as_str()).unwrap_or("unknown");
    sp.finish_ok(&format!("Latest release: {}{}{}", term::bold(), tag, term::reset()));

    let assets = release
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("no assets in GitHub release"))?;

    // ---- 2. Pick the right asset for this platform/arch ----
    let (asset_name, asset_url) = select_llama_cpp_asset(assets)
        .ok_or_else(|| anyhow::anyhow!(
            "No suitable llama.cpp binary found for {}/{}\n\
             Download manually from: https://github.com/ggerganov/llama.cpp/releases",
            std::env::consts::OS, std::env::consts::ARCH
        ))?;

    println!("  {}Asset{}  {}", term::dim(), term::reset(), asset_name);

    // ---- 3. Download the zip into memory ----
    println!("  {}Downloading archive…{}", term::dim(), term::reset());
    let zip_bytes = download_bytes_with_progress(&client, &asset_url).await?;
    println!("\r  {}✓{}  Download complete                                          ", term::bright_green(), term::reset());

    // ---- 4. Extract llama-server from the zip ----
    let sp = term::Spinner::start("Extracting llama-server from archive…");
    extract_llama_server_from_zip(&zip_bytes, &dest)?;
    sp.finish_ok(&format!("Installed: {}{}{}", term::bold(), dest.display(), term::reset()));

    Ok(dest)
}

/// Pick the best zip asset from the GitHub release assets list for the current OS/arch.
/// Returns `(asset_name, download_url)` or `None` if no match found.
fn select_llama_cpp_asset(assets: &[serde_json::Value]) -> Option<(String, String)> {
    let os = std::env::consts::OS;     // "linux" | "macos" | "windows"
    let arch = std::env::consts::ARCH; // "x86_64" | "aarch64"

    // Keywords that must appear in the asset name for os and arch.
    let os_keywords: &[&str] = match os {
        "linux"   => &["linux", "ubuntu"],
        "macos"   => &["macos", "osx"],
        "windows" => &["win"],
        _         => return None,
    };
    let arch_keywords: &[&str] = match arch {
        "x86_64"  => &["x64"],
        "aarch64" => &["arm64", "aarch64"],
        _         => return None,
    };

    // Collect all matching zip assets (case-insensitive).
    let candidates: Vec<(String, String)> = assets
        .iter()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?;
            let url  = a.get("browser_download_url")?.as_str()?;
            if !name.ends_with(".zip") {
                return None;
            }
            let lower = name.to_lowercase();
            // Must match at least one OS keyword AND at least one arch keyword.
            if !os_keywords.iter().any(|k| lower.contains(k)) {
                return None;
            }
            if !arch_keywords.iter().any(|k| lower.contains(k)) {
                return None;
            }
            Some((name.to_owned(), url.to_owned()))
        })
        .collect();

    // Preference order: avx2 (CPU perf) > plain > cuda/vulkan/kompute (avoid GPU-only builds
    // unless they are the only option — user can always re-run if they have a GPU).
    let non_gpu: Vec<_> = candidates
        .iter()
        .filter(|(n, _)| {
            let l = n.to_lowercase();
            !l.contains("cuda") && !l.contains("vulkan") && !l.contains("kompute")
        })
        .collect();

    let pool: Vec<(String, String)> = if non_gpu.is_empty() {
        candidates.clone()
    } else {
        non_gpu.into_iter().cloned().collect()
    };

    pool.iter()
        .find(|(n, _)| n.to_lowercase().contains("avx2"))
        .or_else(|| pool.first())
        .cloned()
}

/// Returns true if `filename` is a platform shared library that should be
/// extracted alongside the main binary:
///   Windows : `.dll`
///   macOS   : `.dylib`
///   Linux   : `.so` or `.so.<version>` (e.g. `libggml.so.0`)
fn is_shared_lib(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    if cfg!(windows) {
        lower.ends_with(".dll")
    } else if cfg!(target_os = "macos") {
        lower.ends_with(".dylib")
    } else {
        // Linux / other Unix: match ".so" exactly or ".so." prefix (versioned)
        lower.ends_with(".so") || lower.contains(".so.")
    }
}

/// Known core shared-lib stems that llama.cpp requires on every platform.
/// Used to detect an incomplete installation in `setup_llamacpp`.
fn core_lib_stems() -> &'static [&'static str] {
    if cfg!(windows) {
        &["ggml.dll", "llama.dll"]
    } else if cfg!(target_os = "macos") {
        &["libggml.dylib", "libllama.dylib"]
    } else {
        &["libggml.so", "libllama.so"]
    }
}

/// Extract `llama-server[.exe]` and all companion shared libraries from a zip
/// archive into `bin_dir`.
///
/// Platform shared library extensions:
///   Windows : `.dll`
///   macOS   : `.dylib`
///   Linux   : `.so` / `.so.<version>`
fn extract_llama_server_from_zip(zip_bytes: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::Read;

    let bin_dir = dest.parent().ok_or_else(|| anyhow::anyhow!("dest has no parent dir"))?;

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow::anyhow!("opening zip archive: {e}"))?;

    let binary_name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let mut found_binary = false;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| anyhow::anyhow!("reading zip entry {i}: {e}"))?;

        if entry.is_dir() {
            continue;
        }

        let entry_name = entry.name().to_owned();
        let file_part = std::path::Path::new(&entry_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();

        let is_binary = file_part.eq_ignore_ascii_case(binary_name);
        let is_lib = is_shared_lib(&file_part);

        if !is_binary && !is_lib {
            continue;
        }

        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| anyhow::anyhow!("reading zip entry '{}': {e}", entry_name))?;

        let out_path = if is_binary { dest.to_path_buf() } else { bin_dir.join(&file_part) };

        // Atomic write: temp file + rename.
        let tmp = out_path.with_extension("_ntk_tmp");
        std::fs::write(&tmp, &bytes)
            .map_err(|e| anyhow::anyhow!("writing '{}': {e}", tmp.display()))?;
        std::fs::rename(&tmp, &out_path)
            .map_err(|e| anyhow::anyhow!("renaming to '{}': {e}", out_path.display()))?;

        // Mark binary and shared libs executable on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| anyhow::anyhow!("setting permissions on '{}': {e}", out_path.display()))?;
        }

        if is_binary {
            found_binary = true;
        }
    }

    if found_binary {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "llama-server binary not found inside the downloaded archive.\n\
             The release format may have changed — download manually from:\n\
             https://github.com/ggerganov/llama.cpp/releases"
        ))
    }
}

/// Download `url` into a `Vec<u8>`, printing a colored progress bar to stdout.
async fn download_bytes_with_progress(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>> {
    use std::io::Write as _;
    use ntk::output::terminal as term;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("download request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("download returned HTTP {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut bytes: Vec<u8> = if total > 0 {
        Vec::with_capacity(total as usize)
    } else {
        Vec::new()
    };

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow::anyhow!("reading download chunk: {e}"))?
    {
        bytes.extend_from_slice(&chunk);
        if total > 0 {
            let pct = bytes.len().saturating_mul(100).checked_div(total as usize).unwrap_or(0);
            let mb = bytes.len() / 1_048_576;
            let total_mb = total as usize / 1_048_576;
            let bar_width = 20usize;
            let filled = (pct * bar_width / 100).min(bar_width);
            let empty = bar_width - filled;
            let bar_filled = "█".repeat(filled);
            let bar_empty  = "░".repeat(empty);
            print!("\r  {}⬇{}  [{}{}{}{}{}]  {}/{} MB  {}%   ",
                term::bright_cyan(), term::reset(),
                term::bright_green(), bar_filled,
                term::dim(), bar_empty, term::reset(),
                mb, total_mb, pct);
            std::io::stdout().flush().ok();
        }
    }

    Ok(bytes)
}

// ---------------------------------------------------------------------------
// GGUF/tokenizer download helper
// ---------------------------------------------------------------------------

/// Download a file from `url` to `dest`, printing a colored progress bar to stdout.
/// Uses `.chunk()` streaming. Writes to a `.tmp` file first then renames atomically.
fn download_file_with_progress(url: &str, dest: &std::path::Path) -> Result<()> {
    use std::io::Write as _;
    use ntk::output::terminal as term;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30 * 60))
            .build()?;

        let mut resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow!("GET {url}: {e}"))?;

        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} fetching {url}", resp.status()));
        }

        let total_bytes = resp.content_length().unwrap_or(0);

        let tmp_path = dest.with_extension("tmp");
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating {}", tmp_path.display()))?;

        let mut downloaded: u64 = 0;
        let bar_width = 20usize;

        while let Some(chunk) = resp.chunk().await.context("reading chunk")? {
            file.write_all(&chunk)?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);

            if total_bytes > 0 {
                let pct = downloaded.saturating_mul(100).checked_div(total_bytes).unwrap_or(0) as usize;
                let mb = downloaded / 1_048_576;
                let total_mb = total_bytes / 1_048_576;
                let filled = (pct * bar_width / 100).min(bar_width);
                let empty = bar_width - filled;
                let bar_filled = "█".repeat(filled);
                let bar_empty  = "░".repeat(empty);
                print!("\r  {}⬇{}  [{}{}{}{}{}]  {}/{} MB  {}%   ",
                    term::bright_cyan(), term::reset(),
                    term::bright_green(), bar_filled,
                    term::dim(), bar_empty, term::reset(),
                    mb, total_mb, pct);
                std::io::stdout().flush().ok();
            } else {
                let mb = downloaded / 1_048_576;
                print!("\r  {}⬇{}  {} MB downloaded…   ", term::bright_cyan(), term::reset(), mb);
                std::io::stdout().flush().ok();
            }
        }

        file.flush()?;
        drop(file);
        std::fs::rename(&tmp_path, dest)
            .with_context(|| format!("renaming to {}", dest.display()))?;
        Ok(())
    })
}

fn run_model_test() -> Result<()> {
    use ntk::compressor::layer3_backend::BackendKind;
    use ntk::detector::OutputType;
    use ntk::output::terminal as term;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut config = ntk::config::load(&cwd).unwrap_or_default();
    // Use generous timeout for interactive test — first inference after model load can be slow.
    config.model.timeout_ms = 120_000;

    let backend = BackendKind::from_config(&config)?;

    println!(
        "{}Backend:{} {}{}{}",
        term::bold(), term::reset(),
        term::bright_cyan(), backend.name(), term::reset()
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // If llama.cpp, start the server first (with a generous timeout for model loading).
    if config.model.llama_server_auto_start {
        if let BackendKind::LlamaCpp(_) = &backend {
            let sp = term::Spinner::start("Starting llama-server …");
            match rt.block_on(backend.start_if_needed()) {
                Ok(_) => sp.finish_ok("llama-server ready"),
                Err(e) => {
                    sp.finish_err(&format!("llama-server failed: {e}"));
                    return Err(e);
                }
            }
        }
    }

    let test_input = "running 42 tests\ntest result: FAILED. 1 failed; 41 passed; 0 ignored\n\nfailures:\n  test_foo::should_return_42 at src/lib.rs:17\n  left: 0\n  right: 42";
    let prompts_dir = ntk::config::resolve_prompts_dir();

    let sp = term::Spinner::start("Running inference …");
    let start = std::time::Instant::now();
    let result = match rt.block_on(backend.compress(test_input, OutputType::Test, &prompts_dir)) {
        Ok(r) => { sp.finish(); r }
        Err(e) => { sp.finish_err(&e.to_string()); return Err(e); }
    };
    let elapsed = start.elapsed();

    let ratio_pct = result.output_tokens.saturating_mul(100)
        .checked_div(result.input_tokens.max(1)).unwrap_or(0);
    let tok_per_s = if elapsed.as_millis() > 0 {
        result.output_tokens as f64 * 1000.0 / elapsed.as_millis() as f64
    } else { 0.0 };

    println!();
    println!(
        "  {}Input  :{} {} tokens",
        term::bold(), term::reset(), result.input_tokens
    );
    println!(
        "  {}Output :{} {} tokens",
        term::bold(), term::reset(), result.output_tokens
    );
    println!(
        "  {}Latency:{} {}{:.0}ms{}",
        term::bold(), term::reset(),
        term::latency_color(elapsed.as_millis() as u64),
        elapsed.as_millis(),
        term::reset()
    );
    println!(
        "  {}Ratio  :{} {}{ratio_pct}%{} of input  {}Speed: {tok_per_s:.2} tok/s{}",
        term::bold(), term::reset(),
        term::ratio_color(ratio_pct), term::reset(),
        term::dim(), term::reset()
    );
    println!();
    println!("{}{}Compressed output:{}",
        term::bold(), term::bright_cyan(), term::reset());
    println!("{}{}{}", term::dim(), "─".repeat(50), term::reset());
    println!("{}", result.output);
    println!("{}{}{}", term::dim(), "─".repeat(50), term::reset());
    Ok(())
}

fn run_model_bench() -> Result<()> {
    use ntk::compressor::layer3_backend::BackendKind;
    use ntk::detector::OutputType;
    use ntk::output::terminal as term;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut config = ntk::config::load(&cwd).unwrap_or_default();
    // Use generous timeout for bench — CPU-only inference on large payloads can be very slow.
    config.model.timeout_ms = 300_000;

    let backend = BackendKind::from_config(&config)?;

    println!(
        "{}{}NTK Layer 3 Benchmark{} — backend: {}{}{}",
        term::bold(), term::bright_cyan(), term::reset(),
        term::cyan(), backend.name(), term::reset()
    );
    println!("{}{}{}", term::dim(), "══════════════════════════════════════════════════", term::reset());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // If llama.cpp, start the server first (no-op for Ollama/Candle).
    if config.model.llama_server_auto_start {
        if let BackendKind::LlamaCpp(_) = &backend {
            let sp = term::Spinner::start("Starting llama-server …");
            match rt.block_on(backend.start_if_needed()) {
                Ok(_) => { sp.finish_ok("llama-server ready"); println!(); }
                Err(e) => { sp.finish_err(&format!("llama-server failed: {e}")); return Err(e); }
            }
        }
    }

    let prompts_dir = ntk::config::resolve_prompts_dir();

    let payloads: &[(&str, OutputType, &str)] = &[
        (
            "running 5 tests\ntest result: ok. 5 passed",
            OutputType::Test,
            "small (test summary)",
        ),
        (
            &"error TS2345: Argument of type 'string' is not assignable.\n\
              error TS2339: Property does not exist on type.\n"
                .repeat(8),
            OutputType::Build,
            "medium (tsc errors x8)",
        ),
        (
            &"[ERROR] 2024-01-01T00:00:00Z Connection refused: redis:6379\n\
              [WARN]  2024-01-01T00:00:01Z Retrying in 5s...\n\
              [INFO]  2024-01-01T00:00:02Z Server started on :8080\n"
                .repeat(20),
            OutputType::Log,
            "large (log lines x60)",
        ),
    ];

    // 1 run per payload to keep total bench time reasonable on CPU-only machines.
    const RUNS: usize = 1;

    // Table header
    println!(
        "{}{}{:<28}  {:>8}  {:>8}  {:>8}  {:>6}  {:>10}{}",
        term::bold(), term::white(),
        "payload", "min ms", "avg ms", "max ms", "ratio", "tok/s",
        term::reset()
    );
    println!("{}{}{}", term::dim(), "─".repeat(74), term::reset());

    for (input, output_type, label) in payloads {
        let mut latencies_ms: Vec<u64> = Vec::with_capacity(RUNS);
        let mut last_result: Option<ntk::compressor::layer3_inference::Layer3Result> = None;

        // Show spinner with real-time elapsed time while processing
        let sp = std::cell::Cell::new(Some(term::BenchSpinner::start(label, input.len())));
        let take_sp = || sp.take();
        let mut error_occurred = false;
        for _ in 0..RUNS {
            let start = std::time::Instant::now();
            match rt.block_on(backend.compress(input, *output_type, &prompts_dir)) {
                Ok(r) => {
                    latencies_ms.push(start.elapsed().as_millis() as u64);
                    last_result = Some(r);
                }
                Err(e) => {
                    if let Some(s) = take_sp() { s.finish(); }
                    println!(
                        "{}{label:<28}{}  {}ERROR: {e}{}",
                        term::dim(), term::reset(), term::bright_red(), term::reset()
                    );
                    error_occurred = true;
                    break;
                }
            }
        }

        if latencies_ms.is_empty() || error_occurred {
            continue;
        }

        if let Some(s) = take_sp() { s.finish(); }

        let min = latencies_ms.iter().copied().min().unwrap_or(0);
        let max = latencies_ms.iter().copied().max().unwrap_or(0);
        let avg = latencies_ms.iter().copied().sum::<u64>()
            .checked_div(latencies_ms.len() as u64).unwrap_or(0);

        let (ratio_pct, tok_per_s) = if let Some(ref r) = last_result {
            let ratio = r.output_tokens.saturating_mul(100)
                .checked_div(r.input_tokens.max(1)).unwrap_or(0);
            let tps = r.output_tokens as f64 * 1000.0 / avg.max(1) as f64;
            (ratio, tps)
        } else {
            (0, 0.0)
        };

        let rc = term::ratio_color(ratio_pct);
        let lc = term::latency_color(avg);
        let rs = term::reset();
        println!(
            "{label:<28}  {lc}{min:>8}{rs}  {lc}{avg:>8}{rs}  {lc}{max:>8}{rs}  {rc}{ratio_pct:>5}%{rs}  {rc}{tok_per_s:>9.2}/s{rs}"
        );
    }

    println!();
    let gpu = ntk::gpu::detect_best_backend();
    println!(
        "{}GPU backend:{} {}{}{}",
        term::bold(), term::reset(), term::cyan(), gpu, term::reset()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Built-in test payloads
// ---------------------------------------------------------------------------

struct TestCase {
    label: &'static str,
    input: String,
    output_type: ntk::detector::OutputType,
    /// Minimum expected compression ratio (0.0–1.0). 0.0 = no requirement.
    min_ratio: f64,
}

fn test_cases() -> Vec<TestCase> {
    use ntk::detector::OutputType;
    vec![
        TestCase {
            label: "cargo test (failures)",
            output_type: OutputType::Test,
            min_ratio: 0.10,
            input: "running 143 tests\n\
                    test result: FAILED. 141 passed; 2 failed; 0 ignored; 0 measured\n\
                    \nfailures:\n\n\
                    ---- auth::test_login FAILED ----\n\
                    thread 'auth::test_login' panicked at 'assertion failed: token.is_valid()'\n\
                    src/auth.rs:42:5\n\n\
                    ---- db::test_connection FAILED ----\n\
                    thread 'db::test_connection' panicked at 'called `Result::unwrap()` on an `Err` value: Os { code: 111, message: \"Connection refused\" }'\n\
                    src/db.rs:88:9\n\n\
                    test auth::test_register ... ok\n\
                    test auth::test_logout   ... ok\n\
                    test db::test_query      ... ok\n"
                .to_owned(),
        },
        TestCase {
            label: "tsc errors",
            output_type: OutputType::Build,
            min_ratio: 0.05,
            input: "src/components/auth/LoginForm.tsx(42,5): error TS2345: Argument of type 'string | undefined' is not assignable to parameter of type 'string'.\n\
                    src/components/auth/LoginForm.tsx(67,12): error TS2339: Property 'userId' does not exist on type 'Session'.\n\
                    src/hooks/useAuth.ts(15,3): error TS2322: Type 'number' is not assignable to type 'string'.\n\
                    src/hooks/useAuth.ts(28,7): error TS7006: Parameter 'event' implicitly has an 'any' type.\n\
                    src/api/client.ts(10,18): error TS2307: Cannot find module 'types/api' or its corresponding type declarations.\n\
                    Found 5 errors in 3 files.\n\
                    Errors  Files\n     2  src/components/auth/LoginForm.tsx\n\
                         2  src/hooks/useAuth.ts\n     1  src/api/client.ts\n"
                .to_owned(),
        },
        TestCase {
            label: "docker logs (repeated lines)",
            output_type: OutputType::Log,
            min_ratio: 0.30,
            input: {
                let mut s = String::new();
                // Identical lines (no timestamp variation) so L1 can deduplicate them.
                for _ in 0..30u32 {
                    s.push_str("[INFO]  Server listening on :8080\n");
                }
                s.push_str("[ERROR] Connection refused: redis:6379\n");
                s.push_str("[WARN]  Retry 1/3 in 5s\n");
                s.push_str("[WARN]  Retry 2/3 in 5s\n");
                s.push_str("[ERROR] Max retries exceeded\n");
                s
            },
        },
        TestCase {
            label: "git diff",
            output_type: OutputType::Diff,
            min_ratio: 0.05,
            input: "diff --git a/src/main.rs b/src/main.rs\n\
                    index 1a2b3c4..5d6e7f8 100644\n\
                    --- a/src/main.rs\n\
                    +++ b/src/main.rs\n\
                    @@ -10,7 +10,7 @@ fn main() {\n \nfn main() {\n\
                    -    println!(\"Hello, world!\");\n\
                    +    println!(\"Hello, NTK!\");\n\
                    }\n\n\
                    // unchanged context line\n\
                    // another unchanged line\n\
                    // yet another unchanged line\n"
                .to_owned(),
        },
    ]
}

fn run_test(with_l3: bool) -> Result<()> {
    use ntk::compressor::{layer1_filter, layer2_tokenizer};

    println!("NTK Compression Tests");
    println!("══════════════════════════════════════════════════");

    let cases = test_cases();
    let mut passed = 0usize;
    let mut failed = 0usize;

    for case in &cases {
        let l1 = layer1_filter::filter(&case.input);
        let l2 = layer2_tokenizer::process(&l1.output)?;

        let original = layer2_tokenizer::count_tokens(&case.input)?;
        let ratio = if original > 0 {
            let saved = original.saturating_sub(l2.compressed_tokens);
            saved as f64 / original as f64
        } else {
            0.0
        };

        // Check: errors must be preserved.
        let errors_preserved = !case.input.contains("FAILED") || l2.output.contains("FAILED")
            || l2.output.contains("failed")
            || l2.output.contains("error");

        let ratio_ok = ratio >= case.min_ratio;
        let ok = errors_preserved && ratio_ok;

        let mark = if ok { "✓" } else { "✗" };
        println!(
            "  {mark} {:<28}  {:>5} → {:>5} tokens  ({:.0}% saved)  L{} lines_removed:{}",
            case.label,
            original,
            l2.compressed_tokens,
            ratio * 100.0,
            if l1.lines_removed > 0 || l2.compressed_tokens < original { "1+2" } else { "0" },
            l1.lines_removed,
        );

        if !ratio_ok {
            println!("    ✗ ratio {:.1}% < minimum {:.1}%", ratio * 100.0, case.min_ratio * 100.0);
        }
        if !errors_preserved {
            println!("    ✗ error keywords lost in compression");
        }

        if ok {
            passed = passed.saturating_add(1);
        } else {
            failed = failed.saturating_add(1);
        }
    }

    if with_l3 {
        println!();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config = ntk::config::load(&cwd).unwrap_or_default();
        let backend = ntk::compressor::layer3_backend::BackendKind::from_config(&config)
            .unwrap_or_else(|_| {
                ntk::compressor::layer3_backend::BackendKind::from_config(&ntk::config::NtkConfig::default())
                    .expect("default Ollama backend")
            });
        println!("Layer 3 ({}):", backend.name());
        let prompts_dir = ntk::config::resolve_prompts_dir();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        for case in &cases {
            let start = std::time::Instant::now();
            match rt.block_on(backend.compress(&case.input, case.output_type, &prompts_dir)) {
                Ok(r) => {
                    let ms = start.elapsed().as_millis();
                    let ratio = if r.input_tokens > 0 {
                        r.output_tokens.saturating_mul(100).checked_div(r.input_tokens).unwrap_or(0)
                    } else { 0 };
                    println!("  ✓ {:<28}  {}ms  {} → {} tokens  ({ratio}% of input)", case.label, ms, r.input_tokens, r.output_tokens);
                    passed = passed.saturating_add(1);
                }
                Err(e) => {
                    println!("  ✗ {:<28}  ERROR: {e}", case.label);
                    failed = failed.saturating_add(1);
                }
            }
        }
    }

    println!();
    println!("Results: {passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn run_bench(runs: usize, with_l3: bool) -> Result<()> {
    use ntk::compressor::{layer1_filter, layer2_tokenizer};
    use std::time::Instant;

    println!("NTK Compression Benchmark  ({runs} runs per payload)");
    println!("══════════════════════════════════════════════════════════════════");
    println!(
        "{:<28}  {:>7}  {:>8}  {:>8}  {:>8}  {:>7}",
        "payload", "tokens", "min µs", "avg µs", "max µs", "ratio"
    );
    println!("{}", "-".repeat(70));

    let cases = test_cases();

    for case in &cases {
        let original = layer2_tokenizer::count_tokens(&case.input)?;
        let mut latencies_us: Vec<u64> = Vec::with_capacity(runs);
        let mut last_compressed = 0usize;

        for _ in 0..runs {
            let start = Instant::now();
            let l1 = layer1_filter::filter(&case.input);
            let l2 = layer2_tokenizer::process(&l1.output)?;
            latencies_us.push(start.elapsed().as_micros() as u64);
            last_compressed = l2.compressed_tokens;
        }

        let min = latencies_us.iter().copied().min().unwrap_or(0);
        let max = latencies_us.iter().copied().max().unwrap_or(0);
        let sum: u64 = latencies_us.iter().copied().sum();
        let avg = sum.checked_div(runs.max(1) as u64).unwrap_or(0);

        let ratio_pct = if original > 0 {
            let saved = original.saturating_sub(last_compressed);
            saved.saturating_mul(100).checked_div(original).unwrap_or(0)
        } else {
            0
        };

        println!(
            "{:<28}  {:>7}  {:>8}  {:>8}  {:>8}  {:>6}%",
            case.label, original, min, avg, max, ratio_pct
        );
    }

    if with_l3 {
        println!();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config = ntk::config::load(&cwd).unwrap_or_default();
        let backend = ntk::compressor::layer3_backend::BackendKind::from_config(&config)
            .unwrap_or_else(|_| {
                ntk::compressor::layer3_backend::BackendKind::from_config(&ntk::config::NtkConfig::default())
                    .expect("default Ollama backend")
            });
        println!("Layer 3 — {} inference  ({runs} runs per payload)", backend.name());
        println!(
            "{:<28}  {:>8}  {:>8}  {:>8}  {:>7}",
            "payload", "min ms", "avg ms", "max ms", "ratio"
        );
        println!("{}", "-".repeat(65));

        let prompts_dir = ntk::config::resolve_prompts_dir();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        for case in &cases {
            let mut l3_latencies: Vec<u64> = Vec::with_capacity(runs);
            let mut last_l3: Option<ntk::compressor::layer3_inference::Layer3Result> = None;

            for _ in 0..runs {
                let start = std::time::Instant::now();
                match rt.block_on(backend.compress(&case.input, case.output_type, &prompts_dir)) {
                    Ok(r) => {
                        l3_latencies.push(start.elapsed().as_millis() as u64);
                        last_l3 = Some(r);
                    }
                    Err(e) => {
                        println!("{:<28}  ERROR: {e}", case.label);
                        break;
                    }
                }
            }

            if l3_latencies.is_empty() {
                continue;
            }
            let min = l3_latencies.iter().copied().min().unwrap_or(0);
            let max = l3_latencies.iter().copied().max().unwrap_or(0);
            let sum: u64 = l3_latencies.iter().copied().sum();
            let avg = sum.checked_div(l3_latencies.len().max(1) as u64).unwrap_or(0);
            let ratio = last_l3.as_ref().map(|r| {
                if r.input_tokens > 0 {
                    r.output_tokens.saturating_mul(100).checked_div(r.input_tokens).unwrap_or(0)
                } else { 0 }
            }).unwrap_or(0);
            println!("{:<28}  {:>8}  {:>8}  {:>8}  {:>6}%", case.label, min, avg, max, ratio);
        }
    } else {
        println!();
        println!("Note: L1+L2 only (pure Rust, no Ollama needed). Add --l3 to include inference.");
    }

    println!();
    println!("GPU backend: {}", ntk::gpu::detect_best_backend());
    Ok(())
}

fn run_discover() -> Result<()> {
    use ntk::compressor::layer2_tokenizer;

    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    let transcripts_dir = home.join(".claude").join("transcripts");

    if !transcripts_dir.exists() {
        println!("No Claude transcripts directory found at {}.", transcripts_dir.display());
        return Ok(());
    }

    // Collect .jsonl transcript files, newest first.
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&transcripts_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();

    if files.is_empty() {
        println!("No transcript files found in {}.", transcripts_dir.display());
        return Ok(());
    }

    // Sort by modification time descending (newest first).
    files.sort_by_key(|p| {
        p.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    files.reverse();

    // Analyse only the most recent transcript.
    let transcript = &files[0];
    let contents = std::fs::read_to_string(transcript)
        .with_context(|| format!("reading {}", transcript.display()))?;

    println!(
        "Analyzing transcript: {}",
        transcript.file_name().unwrap_or_default().to_string_lossy()
    );
    println!();

    // Each line is a JSON object. Look for Bash tool responses without NTK compression.
    let mut opportunities: Vec<(String, usize)> = Vec::new();

    for line in contents.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        // Look for tool result lines: {"type":"tool_result","tool_use_id":...,"content":...}
        // or assistant messages containing tool_use of Bash.
        let output = val
            .get("content")
            .and_then(|c| c.as_str())
            .or_else(|| {
                val.get("tool_response")
                    .and_then(|r| r.get("output"))
                    .and_then(|o| o.as_str())
            });

        let command = val
            .get("tool_input")
            .and_then(|i| i.get("command"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if let Some(output_text) = output {
            // Skip already-compressed short outputs.
            if output_text.len() < 500 {
                continue;
            }
            // Check if NTK already processed it (would contain our marker).
            if output_text.contains("NTK compressed:") {
                continue;
            }
            let tokens = layer2_tokenizer::count_tokens(output_text).unwrap_or(0);
            if tokens > 100 {
                let cmd = if command.is_empty() {
                    "unknown"
                } else {
                    command
                };
                // Estimate savings at ~70% (conservative average for L1+L2).
                let estimated = (tokens as f64 * 0.70).round() as usize;
                opportunities.push((cmd.to_string(), estimated));
            }
        }
    }

    if opportunities.is_empty() {
        println!("No missed compression opportunities found.");
        return Ok(());
    }

    println!("Missed compressions in last session:");
    // Sort by estimated savings descending.
    opportunities.sort_by(|a, b| b.1.cmp(&a.1));
    for (cmd, estimated) in opportunities.iter().take(10) {
        let display_cmd = if cmd.len() > 40 { &cmd[..40] } else { cmd };
        println!(
            "  {:<42}  ~{} tokens (estimated 70% savings)",
            display_cmd, estimated
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn daemon_url() -> Result<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();
    Ok(format!("http://{}:{}", config.daemon.host, config.daemon.port))
}

fn pid_file_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".ntk").join("ntk.pid"))
}

/// Minimal synchronous HTTP GET using reqwest in a blocking context.
fn ureq_get(url: &str) -> Result<String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let text = client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow!("daemon unreachable — is `ntk start` running? ({e})"))?
            .text()
            .await?;
        Ok(text)
    })
}
