use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "ntk",
    version,
    about = "Neural Token Killer — semantic compression for Claude Code"
)]
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

        /// Target Cursor (uses MCP instead of PostToolUse hook —
        /// registers `ntk mcp-server` in ~/.cursor/mcp.json).
        #[arg(long)]
        cursor: bool,

        /// Target Zed (uses MCP via context_servers in Zed's
        /// settings.json — registers `ntk mcp-server`).
        #[arg(long)]
        zed: bool,

        /// Target Continue (uses MCP via the mcpServers array in
        /// ~/.continue/config.json — registers `ntk mcp-server`).
        #[arg(long)]
        r#continue: bool,

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
        /// Also run Layer 3 (local inference) against the daemon.
        #[arg(long)]
        with_l3: bool,
        /// User intent to inject as Layer 4 context (implies --with-l3).
        #[arg(long)]
        context: Option<String>,
        /// L4 prompt format: prefix | xml | goal | json (default: prefix).
        #[arg(long, default_value = "prefix")]
        l4_format: String,
        /// Daemon URL (used when --with-l3 or --context is set).
        #[arg(long, default_value = "http://127.0.0.1:8765")]
        daemon_url: String,
        /// Print per-layer latency, delta tokens, and a preview of each
        /// intermediate output (L1, L2, and — when --with-l3 — L3).
        #[arg(long)]
        verbose: bool,
        /// (POC) Run the input through the RFC-0001 YAML spec-loader
        /// instead of the hardcoded L1/L2 path. Accepts a single rule
        /// file OR a directory (all *.yaml files are composed in
        /// filename order). Still passes through L2 after.
        #[arg(long, value_name = "PATH")]
        spec: Option<PathBuf>,
    },

    /// Manage the local inference model.
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },

    /// Show a combined dashboard: status, session gain, and token savings chart.
    Dashboard,

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

        /// Emit a structured JSON report to stdout (and optionally to
        /// the given path). Intended for hardware-benchmark submissions
        /// — contributors can attach the JSON to a GitHub issue.
        #[arg(long)]
        submit: bool,

        /// When used with --submit, write the report to this path instead
        /// of stdout. The path is printed so the user can attach it.
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Run NTK as a Model Context Protocol (MCP) server over stdio.
    /// Exposes L1+L2 compression as a `compress_output` tool any
    /// MCP-compatible client (Cursor, Zed, Windsurf, Claude Desktop)
    /// can invoke. Self-contained — does not require `ntk start`.
    ///
    /// Clients launch this as a child process; it never prints to
    /// stdout except for JSON-RPC responses. Logs go to stderr.
    #[command(name = "mcp-server")]
    McpServer,

    /// Stream recent compression events from the metrics database.
    /// Useful for debugging "is the hook firing?" during a live session
    /// and for spotting which commands dominate token usage over time.
    Tail {
        /// Follow the tail (poll every --interval ms until Ctrl+C).
        #[arg(short = 'f', long)]
        follow: bool,
        /// Polling interval in milliseconds when --follow is set.
        #[arg(long, default_value = "2000")]
        interval: u64,
        /// Show this many recent rows before starting to follow.
        #[arg(short = 'n', long, default_value = "10")]
        lines: usize,
        /// Only show rows whose command starts with this word (e.g.
        /// "cargo", "git"). Case-sensitive; matches the first token.
        #[arg(long)]
        command: Option<String>,
    },

    /// Prune old rows from the SQLite metrics database (compression_records
    /// and l3_cache) to keep disk usage bounded in long-running installs.
    /// Runs VACUUM after deletion so freed pages are returned to the OS.
    Prune {
        /// Delete rows older than this many days. Default: 30.
        #[arg(long, default_value = "30")]
        older_than: u32,
        /// Show what would be deleted without actually deleting.
        #[arg(long)]
        dry_run: bool,
    },

    /// Show a unified diff between the input file and the output of a
    /// specific compression layer. Useful for pinpointing which rule
    /// fired when a fixture's ratio regresses.
    Diff {
        /// Path to the file to diff.
        file: PathBuf,
        /// Which layer to compare against: l1 | l2 | all (default: all).
        #[arg(long, default_value = "all")]
        layer: String,
        /// Context lines around changes (default: 3, 0 = changes only).
        #[arg(long, default_value = "3")]
        context: usize,
    },
}

#[derive(Subcommand, Debug)]
enum ModelAction {
    /// Interactive wizard: compare backends and configure one.
    Setup,
    /// (Re)download and extract the llama-server binary for this OS +
    /// configured `model.gpu_vendor`. Picks the appropriate llama.cpp
    /// release asset automatically: CUDA for NVIDIA, Vulkan for AMD,
    /// platform-default for Apple (Metal is bundled on macOS).
    #[command(name = "install-server")]
    InstallServer,
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
    Test {
        /// Enable verbose debug output: thread config, model path, raw LLM response,
        /// timing breakdown (startup / prefill / generation), and llama-server args.
        #[arg(long)]
        debug: bool,
    },
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

        Some(Command::Init {
            global,
            opencode,
            cursor,
            zed,
            r#continue,
            auto_patch,
            hook_only,
            show,
            uninstall,
        }) => run_init(
            global, opencode, cursor, zed, r#continue, auto_patch, hook_only, show, uninstall,
        ),

        Some(Command::Start { gpu }) => run_daemon(gpu),

        Some(Command::Stop) => run_stop(),

        Some(Command::Status) => run_status(),

        Some(Command::Metrics) => run_metrics(),

        Some(Command::Graph) => run_graph(),

        Some(Command::Gain) => run_gain(),

        Some(Command::History) => run_history(),

        Some(Command::Config { file }) => run_config(file),

        Some(Command::TestCompress {
            file,
            with_l3,
            context,
            l4_format,
            daemon_url,
            verbose,
            spec,
        }) => run_test_compress(
            &file,
            with_l3,
            context,
            &l4_format,
            &daemon_url,
            verbose,
            spec.as_deref(),
        ),

        Some(Command::Model { action }) => run_model(action),

        Some(Command::Dashboard) => run_dashboard(),

        Some(Command::Discover) => run_discover(),

        Some(Command::Test { l3 }) => run_test(l3),

        Some(Command::Bench {
            runs,
            l3,
            submit,
            output,
        }) => run_bench(runs, l3, submit, output),

        Some(Command::Diff {
            file,
            layer,
            context,
        }) => run_diff(&file, &layer, context),

        Some(Command::McpServer) => ntk::mcp_server::run(),

        Some(Command::Prune {
            older_than,
            dry_run,
        }) => run_prune(older_than, dry_run),

        Some(Command::Tail {
            follow,
            interval,
            lines,
            command,
        }) => run_tail(follow, interval, lines, command),
    }
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

// The install surface needs one boolean per editor target; packing them
// into a struct would obscure the CLI shape without making the function
// simpler. Lint exception is scoped to this single function.
#[allow(clippy::too_many_arguments)]
fn run_init(
    _global: bool,
    opencode: bool,
    cursor: bool,
    zed: bool,
    r#continue: bool,
    auto_patch: bool,
    hook_only: bool,
    show: bool,
    uninstall: bool,
) -> Result<()> {
    use ntk::installer::{EditorTarget, Installer};

    let picked = [opencode, cursor, zed, r#continue]
        .iter()
        .filter(|b| **b)
        .count();
    if picked > 1 {
        return Err(anyhow!(
            "--opencode, --cursor, --zed and --continue are mutually exclusive — pick one editor per install"
        ));
    }

    let editor = if cursor {
        EditorTarget::Cursor
    } else if zed {
        EditorTarget::Zed
    } else if r#continue {
        EditorTarget::Continue
    } else if opencode {
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
        return installer.show_status();
    }
    if uninstall {
        return installer.uninstall();
    }

    // `ntk init` only configures NTK itself. Model backend selection and any
    // Ollama / AI-runtime installation are handled by `ntk model setup`.
    installer.run()
}

fn run_daemon(gpu: bool) -> Result<()> {
    // If the daemon is already running, attach to its live TUI dashboard
    // instead of trying to bind the port again (which would just error out).
    if let Ok(url) = daemon_url() {
        let already_up = ureq_get(&format!("{url}/health")).is_ok();
        if already_up {
            return tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(ntk::output::dashboard::run_attach_dashboard(url));
        }
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_run_daemon(gpu))
}

async fn async_run_daemon(gpu: bool) -> Result<()> {
    use tracing_subscriber::prelude::*;

    let warn_buf: ntk::output::dashboard::WarnBuffer =
        Arc::new(Mutex::new(std::collections::VecDeque::new()));

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .with(ntk::output::dashboard::WarnCaptureLayer::new(Arc::clone(
            &warn_buf,
        )))
        .init();

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd)?;

    if gpu {
        let gpus = ntk::gpu::enumerate_gpus();
        if gpus.is_empty() {
            tracing::warn!("--gpu requested but no discrete GPU was detected; falling back to CPU");
        } else {
            let chosen_id = config.model.cuda_device as usize;
            // `gpus` is non-empty (outer branch), so `.first()` is always Some
            // and this fallback never fails. Prefer a match over `.unwrap()` so
            // the security-gate clippy lint (`-W clippy::unwrap_used`) stays green.
            if let Some(chosen) = gpus.get(chosen_id).or_else(|| gpus.first()) {
                tracing::info!(
                    "GPU inference enabled: {} (device {})",
                    chosen,
                    chosen.device_id
                );
                if gpus.len() > 1 {
                    tracing::info!(
                        "{} GPUs detected — run `ntk model setup` to pick a different one",
                        gpus.len()
                    );
                }
            }
        }
    }

    let host = config.daemon.host.clone();
    let port = config.daemon.port;
    let addr = format!("{host}:{port}");

    // Security: daemon binds to a loopback address by default. The hook
    // pipes every Bash tool result — including env vars, secret paths,
    // and command stdout — into the daemon's /compress endpoint. Exposing
    // that endpoint to the LAN is a leak channel we refuse by default.
    //
    // Opt-out: set NTK_ALLOW_NON_LOOPBACK=1 when you *really* need a
    // non-loopback bind (containerized daemon, remote dev tunnel).
    if !ntk::config::is_loopback_host(&host) {
        let allow = std::env::var("NTK_ALLOW_NON_LOOPBACK")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
            .unwrap_or(false);
        if allow {
            tracing::warn!(
                "NTK_ALLOW_NON_LOOPBACK=1 → daemon will bind to non-loopback host {host}. \
                 Every Bash tool output will be reachable from the network. \
                 Prefer 127.0.0.1 unless you fully control the interface."
            );
        } else {
            return Err(anyhow!(
                "refusing to bind NTK daemon to non-loopback host '{host}'. \
                 Set daemon.host to 127.0.0.1 / localhost / ::1 in ~/.ntk/config.json, \
                 or export NTK_ALLOW_NON_LOOPBACK=1 to override (not recommended)."
            ));
        }
    }

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

    // Build Layer 3 backend chain. A single-element chain preserves the
    // pre-#9 behavior; multi-element chains (configured via
    // `model.backend_chain`) try each backend in order on failure.
    let backend = match ntk::compressor::layer3_backend::BackendChain::from_config(&config) {
        Ok(chain) => {
            tracing::info!("Layer 3 backend chain: {:?}", chain.names());
            Arc::new(chain)
        }
        Err(e) => {
            tracing::warn!("Layer 3 backend init failed, defaulting to Ollama only: {e}");
            // Minimal safety net: build a single-backend chain from the
            // hardcoded Ollama default so the daemon always has something
            // to call. This only runs when config parsing itself fails.
            let fallback = ntk::compressor::layer3_backend::BackendKind::Ollama(
                ntk::compressor::layer3_inference::OllamaClient::new(
                    "http://localhost:11434",
                    2000,
                    "phi3:mini",
                ),
            );
            Arc::new(ntk::compressor::layer3_backend::BackendChain::from_single(
                fallback,
            ))
        }
    };

    // Start subprocess if backend requires it (llama.cpp auto_start).
    // Runs in background so the daemon accepts connections immediately.
    if config.model.llama_server_auto_start {
        let backend_bg = Arc::clone(&backend);
        tokio::spawn(async move {
            // BackendChain.start_if_needed() logs per-backend failures
            // internally and never errors the whole chain — one llama.cpp
            // miss shouldn't prevent the Ollama primary from serving.
            backend_bg.start_if_needed().await;
            tracing::info!("llama-server ready");
        });
    }

    let started_at = std::time::Instant::now();
    let metrics = Arc::new(Mutex::new(ntk::metrics::MetricsStore::new()));
    let backend_name = backend.name().to_owned();

    // Build a human-readable model info line shown in the dashboard header.
    // Format: "<model_name> <quantization>  [GPU] or [CPU]"
    let compute_mode = if config.model.gpu_layers != 0 {
        "GPU"
    } else {
        "CPU"
    };
    let model_info = format!(
        "{} {}  [{}]",
        config.model.model_name, config.model.quantization, compute_mode
    );

    // Pin the L2 tokenizer family once at startup so every /compress call
    // reuses the same BPE without re-parsing the vocab. Default is
    // cl100k_base; o200k_base is more accurate for Claude 3.5+/GPT-4o.
    let tk = ntk::compressor::layer2_tokenizer::TokenizerKind::from_config_str(
        &config.compression.tokenizer,
    );
    ntk::compressor::layer2_tokenizer::set_tokenizer(tk);
    tracing::info!("Layer 2 tokenizer: {:?}", tk);

    // Auth: ensure the daemon has a shared-secret token before accepting
    // any request. Generated on first start, persisted at
    // `$HOME/.ntk/.token` with 0600 permissions on Unix.
    let auth_token: String = if ntk::security::auth_disabled() {
        tracing::warn!(
            "NTK_DISABLE_AUTH=1 — daemon accepts unauthenticated requests. \
             This is intended for debugging only; unset in production."
        );
        String::new()
    } else {
        let t = ntk::security::load_or_create_token()
            .context("failed to load/create daemon auth token")?;
        let path = ntk::security::token_path()?;
        tracing::info!("auth token loaded from {}", path.display());
        t
    };

    // Experimental: pre-load the POC spec rulesets if the operator
    // pointed us at a rules path. `NTK_SPEC_RULES` overrides the
    // config field for A/B experimentation without editing JSON.
    let spec_rules = match ntk::server::resolve_spec_rules_path(&config) {
        Some(path) => match ntk::compressor::spec_loader::load_rules_from_path(&path) {
            Ok(files) => {
                tracing::info!(
                    "spec_rules: loaded {} ruleset(s) from {}",
                    files.len(),
                    path.display()
                );
                files
            }
            Err(e) => {
                tracing::warn!(
                    "spec_rules: path {} unusable, falling back to hardcoded L1 only: {e}",
                    path.display()
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    let state = ntk::server::AppState {
        config: Arc::new(config),
        metrics: Arc::clone(&metrics),
        db,
        backend,
        started_at,
        warn_log: Arc::clone(&warn_buf),
        addr: addr.clone(),
        backend_name: backend_name.clone(),
        model_info: model_info.clone(),
        auth_token: Arc::new(auth_token),
        spec_rules: Arc::new(spec_rules),
    };

    let router = ntk::server::build_router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("NTK daemon listening on {addr}");

    // Write PID file so `ntk stop` can terminate us. Done after bind so a port
    // conflict (second daemon attempt) does not overwrite the first daemon's PID.
    let pid_path = pid_file_path()?;
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&pid_path, std::process::id().to_string()) {
        tracing::warn!("failed to write PID file at {}: {e}", pid_path.display());
    }

    // Shutdown channel: either the OS sends SIGINT (Ctrl+C in normal mode)
    // or the dashboard detects Ctrl+C as a raw key event and sends `true`.
    // Both paths converge here so the server and dashboard stop together.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Clone so the dashboard can trigger shutdown (raw mode Ctrl+C path).
    let dashboard_trigger_tx = shutdown_tx.clone();
    // Clone so the server's graceful shutdown can watch the channel.
    let mut shutdown_rx_server = shutdown_rx.clone();

    // Spawn the live dashboard in a separate task.
    let dashboard_handle = tokio::spawn(ntk::output::dashboard::run_live_dashboard(
        Arc::clone(&metrics),
        warn_buf,
        started_at,
        addr.clone(),
        backend_name,
        model_info,
        shutdown_rx,
        dashboard_trigger_tx,
    ));

    // Serve HTTP until either:
    //   • OS Ctrl+C  → SIGINT → tokio::signal::ctrl_c() fires
    //   • Dashboard Ctrl+C key event → shutdown_rx_server fires
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async {
                    loop {
                        if shutdown_rx_server.changed().await.is_err() { break; }
                        if *shutdown_rx_server.borrow() { break; }
                    }
                } => {}
            }
            let _ = shutdown_tx.send(true); // ensure dashboard also stops
        })
        .await?;

    // Wait for the dashboard to finish restoring the terminal before we exit.
    let _ = dashboard_handle.await;

    // Remove PID file on graceful shutdown so a later `ntk status` does not
    // report "running" for a dead process.
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

fn run_stop() -> Result<()> {
    let pid_path = pid_file_path()?;

    // 1. Fast path — read PID file written at daemon start.
    let pid_from_file: Option<u32> = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok());

    // 2. Fallback — scan system sockets for a process listening on our port
    //    (handles orphans from older versions that did not write the PID file,
    //    or daemons killed with SIGKILL that left a stale file).
    let pid_from_socket = pid_listening_on_daemon_port();

    let pid = pid_from_file.or(pid_from_socket);

    let Some(pid) = pid else {
        println!("NTK daemon is not running.");
        let _ = std::fs::remove_file(&pid_path);
        return Ok(());
    };

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
            if handle.is_null() {
                return Err(anyhow!(
                    "cannot open process (PID {pid}) — already stopped?"
                ));
            }
            TerminateProcess(handle, 0);
        }
        println!("Terminated NTK daemon (PID {pid}).");
    }

    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

/// Find the PID of whichever process is listening on the NTK daemon port.
/// Returns `None` when no process is bound to the port (daemon is down).
///
/// Implementation: shells out to a platform-native tool rather than pulling
/// in a heavy cross-platform socket-enumeration crate.
fn pid_listening_on_daemon_port() -> Option<u32> {
    let port = default_daemon_port();
    let port_str = port.to_string();

    #[cfg(windows)]
    {
        // PowerShell is always present on Windows. Get-NetTCPConnection is
        // reliable across Win10+.  Fall back to netstat if PowerShell fails.
        let ps = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-NetTCPConnection -LocalPort {port} -State Listen -ErrorAction SilentlyContinue | \
                     Select-Object -First 1 -ExpandProperty OwningProcess)"
                ),
            ])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&ps.stdout).trim().to_owned();
        if let Ok(pid) = s.parse::<u32>() {
            return Some(pid);
        }

        // netstat fallback: parse "TCP 127.0.0.1:8765 ... LISTENING <PID>"
        let out = std::process::Command::new("netstat")
            .args(["-ano"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            if line.contains(&format!(":{port_str}")) && line.contains("LISTENING") {
                if let Some(tok) = line.split_whitespace().last() {
                    if let Ok(pid) = tok.parse::<u32>() {
                        return Some(pid);
                    }
                }
            }
        }
        None
    }

    #[cfg(unix)]
    {
        // Prefer `lsof -ti:PORT` — single PID per line.
        if let Ok(out) = std::process::Command::new("lsof")
            .args(["-ti", &format!(":{port_str}"), "-sTCP:LISTEN"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if let Ok(pid) = s.parse::<u32>() {
                return Some(pid);
            }
        }
        // Fallback: `ss -lntp` or `fuser`.
        if let Ok(out) = std::process::Command::new("ss").args(["-lntp"]).output() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if line.contains(&format!(":{port_str}")) && line.contains("LISTEN") {
                    // Extract pid=NNN from "users:((\"prog\",pid=12345,fd=...))"
                    if let Some(idx) = line.find("pid=") {
                        let rest = &line[idx.saturating_add(4)..];
                        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(pid) = num.parse::<u32>() {
                            return Some(pid);
                        }
                    }
                }
            }
        }
        None
    }
}

/// Reads the daemon port from the loaded config, falling back to the
/// built-in default (8765) if anything goes wrong.
fn default_daemon_port() -> u16 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ntk::config::load(&cwd)
        .map(|c| c.daemon.port)
        .unwrap_or(8765)
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

    let backend = gpu::resolve_configured_backend(
        config.model.gpu_layers,
        config.model.gpu_vendor,
        config.model.cuda_device,
    );

    use ntk::output::terminal as term;

    let ok = term::ok_mark();
    let err = term::err_mark();

    term::print_header("NTK Status", "══════════════════════════════════════════");

    let daemon_icon = if daemon_ok { &ok } else { &err };
    println!("  Daemon       : {daemon_icon} {daemon_info}");
    println!("  Endpoint     : {}{url}{}", term::dim(), term::reset());
    println!(
        "  GPU backend  : {}{backend}{}",
        term::cyan(),
        term::reset()
    );
    println!(
        "  Config       : {}{}{}",
        term::dim(),
        ntk::config::global_config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_owned()),
        term::reset()
    );
    println!();

    let provider_name = match config.model.provider {
        ntk::config::ModelProvider::Ollama => "ollama",
        ntk::config::ModelProvider::Candle => "candle",
        ntk::config::ModelProvider::LlamaCpp => "llama.cpp",
    };
    println!(
        "{}{}Model ({}):{}",
        term::bold(),
        term::bright_cyan(),
        provider_name,
        term::reset()
    );
    println!(
        "  Configured : {}{}{}",
        term::cyan(),
        config.model.model_name,
        term::reset()
    );
    println!("  Quantize   : {}", config.model.quantization);
    if ollama_ok {
        println!(
            "  Ollama     : {} reachable  ({}{}{})",
            ok,
            term::dim(),
            config.model.ollama_url,
            term::reset()
        );
        if model_list.is_empty() {
            println!(
                "  Available  : {}(none — run `ntk model pull`){}",
                term::yellow(),
                term::reset()
            );
        } else {
            println!("  Available  :");
            for m in &model_list {
                let is_active = m.contains(&config.model.model_name)
                    || config
                        .model
                        .model_name
                        .contains(m.split(':').next().unwrap_or(""));
                if is_active {
                    println!(
                        "    {} {}{m}{} {}◀ active{}",
                        ok,
                        term::bright_green(),
                        term::reset(),
                        term::dim(),
                        term::reset()
                    );
                } else {
                    println!("    {} {}{m}{}", term::dim(), term::reset(), term::reset());
                }
            }
        }
    } else if daemon_ok {
        println!(
            "  Ollama     : {} unreachable — L3 falls back to L1+L2",
            err
        );
    } else {
        println!(
            "  Ollama     : {}(daemon not running){}",
            term::dim(),
            term::reset()
        );
    }
    println!();

    let on_off = |v: bool| -> String {
        if v {
            format!(
                "{}{}on{}",
                term::bold(),
                term::bright_green(),
                term::reset()
            )
        } else {
            format!("{}off{}", term::dim(), term::reset())
        }
    };

    println!(
        "{}{}Compression:{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!(
        "  L1 filter   : {}",
        on_off(config.compression.layer1_enabled)
    );
    println!(
        "  L2 tokenize : {}",
        on_off(config.compression.layer2_enabled)
    );
    println!(
        "  L3 infer    : {}",
        on_off(config.compression.layer3_enabled)
    );
    println!(
        "  L3 threshold: {}{} tokens{}",
        term::cyan(),
        config.compression.inference_threshold_tokens,
        term::reset()
    );
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
        println!(
            "[ntk graph] No metrics database found. Start the daemon and run some commands first."
        );
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
    let url = match daemon_url() {
        Ok(u) => u,
        Err(_) => {
            println!("NTK: 0 tokens saved across 0 compressions (0% avg)");
            println!("[ntk gain] daemon unreachable — start with: ntk start");
            return Ok(());
        }
    };

    // Fetch all session records for the bar chart.
    let records: Vec<ntk::metrics::CompressionRecord> = match ureq_get(&format!("{url}/records")) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_default(),
        Err(_) => vec![],
    };

    if records.is_empty() {
        // No data yet — try to fetch a summary from the daemon.
        match ureq_get(&format!("{url}/metrics")) {
            Ok(response) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&response) {
                    let saved = val["total_tokens_saved"].as_u64().unwrap_or(0);
                    let calls = val["total_compressions"].as_u64().unwrap_or(0);
                    let pct = val["average_ratio"].as_f64().unwrap_or(0.0) * 100.0;
                    println!(
                        "NTK: {saved} tokens saved across {calls} compressions ({pct:.0}% avg)"
                    );
                } else {
                    println!("NTK: 0 tokens saved across 0 compressions (0% avg)");
                    println!("[ntk gain] daemon unreachable — start with: ntk start");
                }
            }
            Err(_) => {
                // Daemon not running — print the zero-state message.
                println!("NTK: 0 tokens saved across 0 compressions (0% avg)");
                println!("[ntk gain] daemon unreachable — start with: ntk start");
            }
        }
        return Ok(());
    }

    // Show the bar chart (includes footer with totals).
    ntk::output::graph::print_bar_chart(&records);

    // RTK-compatible one-liner below the chart (for piping/scripting).
    let saved: u64 = records
        .iter()
        .map(|r| r.original_tokens.saturating_sub(r.compressed_tokens) as u64)
        .sum();
    let calls = records.len() as u64;
    let avg_pct = if records.iter().map(|r| r.original_tokens).sum::<usize>() > 0 {
        let orig: u64 = records.iter().map(|r| r.original_tokens as u64).sum();
        saved as f64 / orig as f64 * 100.0
    } else {
        0.0
    };
    println!();
    println!("NTK: {saved} tokens saved across {calls} compressions ({avg_pct:.0}% avg)");

    Ok(())
}

fn run_dashboard() -> Result<()> {
    // ── Status ───────────────────────────────────────────────────────────
    run_status()?;

    // ── Session gain (live from daemon /metrics) ──────────────────────
    println!();
    use ntk::output::terminal as term;
    println!(
        "{}{}Session Gain:{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    let url = daemon_url()?;
    match ureq_get(&format!("{url}/metrics")) {
        Ok(response) => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&response) {
                let saved = val["total_tokens_saved"].as_u64().unwrap_or(0);
                let calls = val["total_compressions"].as_u64().unwrap_or(0);
                let pct = val["average_ratio"].as_f64().unwrap_or(0.0) * 100.0;
                println!(
                    "  {}{saved}{} tokens saved  ·  {}{calls}{} compressions  ·  {}{pct:.0}%{} avg",
                    term::yellow(),
                    term::reset(),
                    term::yellow(),
                    term::reset(),
                    term::cyan(),
                    term::reset(),
                );
            }
        }
        Err(_) => println!("  {}(daemon not reachable){}", term::dim(), term::reset()),
    }

    // ── Historical bar chart ─────────────────────────────────────────
    println!();
    run_graph()
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

/// Maximum preview lines shown per layer in `--verbose` mode.
/// Bounded to keep output readable on large fixtures.
const VERBOSE_PREVIEW_LINES: usize = 20;

fn print_verbose_section(
    title: &str,
    content: &str,
    tokens: usize,
    prev_tokens: Option<usize>,
    lines: usize,
    latency: Option<std::time::Duration>,
    note: Option<String>,
) {
    let latency_str = latency.map_or_else(|| "—".to_string(), |d| format!("{} ms", d.as_millis()));
    let delta_str = match prev_tokens {
        Some(prev) if prev > 0 => {
            let saved = prev.saturating_sub(tokens);
            let pct = (saved as f64 / prev as f64) * 100.0;
            format!("{tokens} tokens ({pct:+.1}% vs prev)")
        }
        _ => format!("{tokens} tokens"),
    };

    println!();
    println!("┌─ {title} ──────────────────────");
    println!("│ {delta_str}, {lines} lines, {latency_str}");
    if let Some(n) = note {
        println!("│ {n}");
    }
    println!("│ ── preview (first {VERBOSE_PREVIEW_LINES} lines) ──");
    for line in content.lines().take(VERBOSE_PREVIEW_LINES) {
        println!("│ {line}");
    }
    if content.lines().count() > VERBOSE_PREVIEW_LINES {
        println!(
            "│ … ({} more lines)",
            content
                .lines()
                .count()
                .saturating_sub(VERBOSE_PREVIEW_LINES)
        );
    }
    println!("└─────────────────────────────────");
}

/// Run `ntk test-compress --spec <path>` — compose every rule file at
/// `spec_path` (file OR dir of *.yaml) and apply the merged ruleset
/// to the input. L2 runs on the result. Invariant rejections are
/// reported; non-zero rejection count exits non-zero so CI can gate.
fn run_test_compress_spec(
    file: &std::path::Path,
    spec_path: &std::path::Path,
    verbose: bool,
) -> Result<()> {
    use ntk::compressor::{layer2_tokenizer, spec_loader};
    use std::time::Instant;

    let canonical = file
        .canonicalize()
        .map_err(|e| anyhow!("cannot read {}: {e}", file.display()))?;
    let input = std::fs::read_to_string(&canonical)
        .map_err(|e| anyhow!("reading {}: {e}", canonical.display()))?;
    let original_tokens = layer2_tokenizer::count_tokens(&input)?;

    // Resolve --spec to one or more rule files. File arg is treated
    // singly; directory arg loads every *.yaml inside in sorted order
    // (deterministic composition).
    let rule_files: Vec<std::path::PathBuf> = if spec_path.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(spec_path)
            .map_err(|e| anyhow!("reading dir {}: {e}", spec_path.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
            .collect();
        v.sort();
        v
    } else {
        vec![spec_path.to_path_buf()]
    };

    if rule_files.is_empty() {
        return Err(anyhow!(
            "no *.yaml rule files found at {}",
            spec_path.display()
        ));
    }

    let t_spec = Instant::now();
    let mut current = input.clone();
    let mut total_applied: Vec<String> = Vec::new();
    let mut total_rejected: Vec<String> = Vec::new();
    for rf in &rule_files {
        let loaded =
            spec_loader::load_rule_file(rf).map_err(|e| anyhow!("{}: {e}", rf.display()))?;
        let result = spec_loader::apply_rule_file(&current, &loaded);
        current = result.output;
        total_applied.extend(result.applied);
        total_rejected.extend(result.invariant_rejected);
    }
    let spec_elapsed = t_spec.elapsed();
    let after_spec_tokens = layer2_tokenizer::count_tokens(&current)?;

    let t_l2 = Instant::now();
    let l2 = layer2_tokenizer::process(&current)?;
    let l2_elapsed = t_l2.elapsed();

    let saved = original_tokens.saturating_sub(l2.compressed_tokens);
    let ratio_pct = if original_tokens > 0 {
        (saved as f64 / original_tokens as f64) * 100.0
    } else {
        0.0
    };

    if verbose {
        println!(
            "Spec path:          {} ({} rule file{})",
            spec_path.display(),
            rule_files.len(),
            if rule_files.len() == 1 { "" } else { "s" }
        );
        println!("Original tokens:    {original_tokens}");
        println!(
            "After spec tokens:  {after_spec_tokens}  ({} ms)",
            spec_elapsed.as_millis()
        );
        println!(
            "After L2 tokens:    {} ({} ms)",
            l2.compressed_tokens,
            l2_elapsed.as_millis()
        );
        println!("Spec+L2 compression: {ratio_pct:.1}%");
        println!("Rules applied:       {}", total_applied.join(", "));
        if !total_rejected.is_empty() {
            println!(
                "Rules rejected by invariants: {}",
                total_rejected.join(", ")
            );
        }
        println!();
        println!("--- Compressed output (spec → L2) ---");
        println!("{}", l2.output);
    } else {
        println!("File:              {}", canonical.display());
        println!("Spec rulesets:     {}", rule_files.len());
        println!("Original tokens:   {original_tokens}");
        println!("After spec tokens: {after_spec_tokens}");
        println!("After L2 tokens:   {}", l2.compressed_tokens);
        println!("Spec+L2 compression: {ratio_pct:.1}%");
        if !total_rejected.is_empty() {
            println!("Invariant rejections: {}", total_rejected.join(", "));
        }
        println!();
        println!("--- Compressed output (spec → L2) ---");
        println!("{}", l2.output);
    }

    // Non-zero exit when a rule was rejected so CI can gate.
    if !total_rejected.is_empty() {
        return Err(anyhow!(
            "{} rule(s) rejected by invariant check",
            total_rejected.len()
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_test_compress(
    file: &std::path::Path,
    with_l3: bool,
    context: Option<String>,
    l4_format: &str,
    daemon_url: &str,
    verbose: bool,
    spec: Option<&std::path::Path>,
) -> Result<()> {
    use ntk::compressor::{layer1_filter, layer2_tokenizer};
    use std::time::Instant;

    // POC path: when --spec is provided, replace L1 with the RFC-0001
    // YAML ruleset engine. L2 still runs on the result — the spec
    // only subsumes the line-level transformations L1 handles.
    if let Some(spec_path) = spec {
        return run_test_compress_spec(file, spec_path, verbose);
    }

    let canonical = file
        .canonicalize()
        .map_err(|e| anyhow!("cannot read {}: {e}", file.display()))?;

    let input = std::fs::read_to_string(&canonical)
        .map_err(|e| anyhow!("reading {}: {e}", canonical.display()))?;

    let original_tokens = layer2_tokenizer::count_tokens(&input)?;
    let input_lines = input.lines().count();

    let t_l1 = Instant::now();
    let l1 = layer1_filter::filter(&input);
    let l1_elapsed = t_l1.elapsed();
    let l1_tokens = layer2_tokenizer::count_tokens(&l1.output)?;

    let t_l2 = Instant::now();
    let l2 = layer2_tokenizer::process(&l1.output)?;
    let l2_elapsed = t_l2.elapsed();

    let ratio_l2 = if original_tokens > 0 {
        let saved = original_tokens.saturating_sub(l2.compressed_tokens);
        (saved as f64 / original_tokens as f64) * 100.0
    } else {
        0.0
    };

    if verbose {
        print_verbose_section(
            "Input",
            &input,
            original_tokens,
            None,
            input_lines,
            None,
            None,
        );
        let l1_lines = l1.output.lines().count();
        let l1_applied = if l1.applied_rules.is_empty() {
            "none".to_string()
        } else {
            l1.applied_rules.join(", ")
        };
        let l1_note = Some(format!(
            "Applied: {l1_applied} · {} lines removed total",
            l1.lines_removed
        ));
        print_verbose_section(
            "L1 output (regex/filter)",
            &l1.output,
            l1_tokens,
            Some(original_tokens),
            l1_lines,
            Some(l1_elapsed),
            l1_note,
        );
        let l2_lines = l2.output.lines().count();
        let l2_applied = if l2.applied_rules.is_empty() {
            "none".to_string()
        } else {
            l2.applied_rules.join(", ")
        };
        print_verbose_section(
            "L2 output (tokenizer)",
            &l2.output,
            l2.compressed_tokens,
            Some(l1_tokens),
            l2_lines,
            Some(l2_elapsed),
            Some(format!("Applied: {l2_applied}")),
        );
    } else {
        println!("File:              {}", canonical.display());
        println!("Original tokens:   {original_tokens}");
        println!("L1 lines removed:  {}", l1.lines_removed);
        println!("After L2 tokens:   {}", l2.compressed_tokens);
        println!("L1+L2 compression: {ratio_l2:.1}%");
    }

    // Fire a full-pipeline request through the daemon when requested.
    let want_full_pipeline = with_l3 || context.is_some();
    if want_full_pipeline {
        println!();
        println!("--- Running L3/L4 via daemon at {daemon_url} ---");

        let mut payload = serde_json::json!({
            "output": input,
            "command": "ntk test-compress",
            "cwd": canonical.parent().map_or(".".to_string(), |p| p.display().to_string()),
        });
        if let Some(ctx) = &context {
            payload["context"] = serde_json::Value::String(ctx.clone());
        }

        // Daemon inherits NTK_L4_FORMAT from its own env; we hint the user instead
        // of spawning a new daemon here (which would race with the one on :8765).
        if !matches!(l4_format, "prefix" | "xml" | "xmlwrap" | "goal" | "json") {
            println!("warning: unknown --l4-format '{l4_format}', daemon default will be used");
        } else if l4_format != "prefix" {
            println!(
                "note: set NTK_L4_FORMAT={l4_format} before `ntk start` for this format to apply"
            );
        }

        let url = format!("{}/compress", daemon_url.trim_end_matches('/'));
        // Shared-secret token — /compress is a protected route.
        let token = ntk::security::load_or_create_token()
            .ok()
            .unwrap_or_default();
        let rt = tokio::runtime::Runtime::new()?;
        let body: serde_json::Value = rt.block_on(async {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()?;
            let mut req = client.post(&url).json(&payload);
            if !token.is_empty() {
                req = req.header(ntk::security::TOKEN_HEADER, &token);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| anyhow!("daemon unreachable at {url}: {e}. Run `ntk start` first."))?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("daemon returned {status}: {text}"));
            }
            resp.json::<serde_json::Value>()
                .await
                .map_err(|e| anyhow!("invalid JSON response: {e}"))
        })?;

        let layer = body.get("layer").and_then(|v| v.as_u64()).unwrap_or(0);
        let tokens_after = body
            .get("tokens_after")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ratio = body
            .get("ratio")
            .and_then(|v| v.as_f64())
            .map(|r| r * 100.0)
            .unwrap_or(0.0);
        let compressed = body
            .get("compressed")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if verbose {
            let l3_latency_ms = body.pointer("/latency_ms/l3").and_then(|v| v.as_u64());
            let tokens_after_l2 = body
                .get("tokens_after_l2")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let note = if let Some(ctx) = &context {
                let preview: String = ctx.chars().take(80).collect();
                let suffix = if ctx.chars().count() > 80 { "…" } else { "" };
                Some(format!(
                    "L4 context injected: \"{preview}{suffix}\" (format: {l4_format})"
                ))
            } else {
                Some(format!(
                    "L4 context: none; L3 invoked: layer reached L{layer}"
                ))
            };
            print_verbose_section(
                "L3 output (local inference via daemon)",
                compressed,
                tokens_after as usize,
                tokens_after_l2,
                compressed.lines().count(),
                l3_latency_ms.map(std::time::Duration::from_millis),
                note,
            );
            println!();
            println!("Total: {original_tokens} → {tokens_after} tokens ({ratio:.1}%)");
        } else {
            println!("Layer reached:     L{layer}");
            println!("Final tokens:      {tokens_after}");
            println!("Total compression: {ratio:.1}%");
            println!();
            println!("--- Compressed output ---");
            println!("{compressed}");
        }
    } else {
        println!();
        println!("--- Compressed output (L1+L2) ---");
        println!("{}", l2.output);
        println!();
        println!(
            "tip: pass --with-l3 (or --context \"<intent>\") to also run Layer 3/4 via the daemon."
        );
    }
    Ok(())
}

fn run_tail(
    follow: bool,
    interval_ms: u64,
    lines: usize,
    command_filter: Option<String>,
) -> Result<()> {
    use ntk::metrics::MetricsDb;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();
    let db_path = config.storage_path_expanded();

    if !db_path.exists() {
        println!(
            "No metrics database at {} — no rows to tail yet.\n\
             Start the daemon and run some Bash tool calls first.",
            db_path.display()
        );
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let db = MetricsDb::init(&db_path).await?;

        // Initial page: most-recent `lines` rows. SQL returns DESC;
        // flip to chronological before printing so the live tail below
        // appends naturally.
        let mut cursor = {
            let hist = db.history(lines).await?;
            for row in &hist {
                print_tail_line_history(row);
            }
            db.max_record_id().await?
        };

        if !follow {
            return Ok::<(), anyhow::Error>(());
        }

        tracing::info!(
            "tailing {} (every {interval_ms}ms, Ctrl+C to stop)",
            db_path.display()
        );

        // Poll loop. Ctrl+C propagates through tokio::signal::ctrl_c.
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_millis(interval_ms.max(200)));
        ticker.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!();
                    return Ok(());
                }
                _ = ticker.tick() => {
                    let rows = db
                        .records_since(cursor, command_filter.as_deref(), 500)
                        .await?;
                    for r in &rows {
                        print_tail_line(r);
                        if r.id > cursor {
                            cursor = r.id;
                        }
                    }
                }
            }
        }
    })?;

    Ok(())
}

fn print_tail_line(r: &ntk::metrics::TailRow) {
    // created_at is ISO; keep only the HH:MM:SS slice for terseness.
    let ts = r
        .created_at
        .split(['T', ' '])
        .nth(1)
        .and_then(|t| t.split('.').next())
        .unwrap_or(&r.created_at);
    let ratio_pct = if r.original_tokens > 0 {
        let saved = r.original_tokens.saturating_sub(r.compressed_tokens);
        saved
            .saturating_mul(100)
            .checked_div(r.original_tokens)
            .unwrap_or(0)
    } else {
        0
    };
    println!(
        "{ts} | {cmd:<30} | {bi:>5}→{ao:<5} ({pct:>3}%) | {lat:>5} ms | L{layer}",
        cmd = truncate(&r.command, 30),
        bi = r.original_tokens,
        ao = r.compressed_tokens,
        pct = ratio_pct,
        lat = r.latency_ms,
        layer = r.layer_used,
    );
}

fn print_tail_line_history(r: &ntk::metrics::HistoryRow) {
    let ts = r
        .created_at
        .split(['T', ' '])
        .nth(1)
        .and_then(|t| t.split('.').next())
        .unwrap_or(&r.created_at);
    let ratio_pct = if r.original_tokens > 0 {
        let saved = r.original_tokens.saturating_sub(r.compressed_tokens);
        saved
            .saturating_mul(100)
            .checked_div(r.original_tokens)
            .unwrap_or(0)
    } else {
        0
    };
    println!(
        "{ts} | {cmd:<30} | {bi:>5}→{ao:<5} ({pct:>3}%) | {lat:>5} ms | L{layer}",
        cmd = truncate(&r.command, 30),
        bi = r.original_tokens,
        ao = r.compressed_tokens,
        pct = ratio_pct,
        lat = r.latency_ms,
        layer = r.layer_used,
    );
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

fn run_prune(older_than: u32, dry_run: bool) -> Result<()> {
    use ntk::metrics::MetricsDb;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();
    let db_path = config.storage_path_expanded();

    if !db_path.exists() {
        println!(
            "No metrics database at {} — nothing to prune.",
            db_path.display()
        );
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let db = MetricsDb::init(&db_path).await?;
        let rec_before = db.records_size().await?;
        let cache_before = db.l3_cache_size().await?;
        println!("Database: {}", db_path.display());
        println!("  compression_records: {rec_before}");
        println!("  l3_cache entries:    {cache_before}");
        println!();

        if dry_run {
            println!(
                "--dry-run: would prune rows older than {older_than} day(s).\n\
                 Re-run without --dry-run to actually delete + VACUUM."
            );
            return Ok::<(), anyhow::Error>(());
        }

        let (recs, cache) = db.prune_older_than(older_than).await?;
        println!("Pruned (older than {older_than} days):");
        println!("  compression_records: {recs} row(s) deleted");
        println!("  l3_cache entries:    {cache} row(s) deleted");
        println!();
        println!("VACUUM completed.");

        let rec_after = db.records_size().await?;
        let cache_after = db.l3_cache_size().await?;
        println!();
        println!("After prune:");
        println!("  compression_records: {rec_after}");
        println!("  l3_cache entries:    {cache_after}");
        Ok(())
    })?;

    Ok(())
}

fn run_diff(file: &std::path::Path, layer: &str, context: usize) -> Result<()> {
    use ntk::compressor::{layer1_filter, layer2_tokenizer};
    use similar::{ChangeTag, TextDiff};

    let canonical = file
        .canonicalize()
        .map_err(|e| anyhow!("cannot read {}: {e}", file.display()))?;
    let input = std::fs::read_to_string(&canonical)
        .map_err(|e| anyhow!("reading {}: {e}", canonical.display()))?;

    let layer = layer.to_lowercase();
    let show_l1 = matches!(layer.as_str(), "l1" | "all");
    let show_l2 = matches!(layer.as_str(), "l2" | "all");
    if !show_l1 && !show_l2 {
        return Err(anyhow!(
            "unknown --layer '{layer}' (expected: l1 | l2 | all)"
        ));
    }

    let l1 = layer1_filter::filter(&input);
    let l2 = if show_l2 {
        Some(layer2_tokenizer::process(&l1.output)?)
    } else {
        None
    };

    println!("File: {}", canonical.display());
    println!();

    if show_l1 {
        print_unified_diff("Input vs L1", &input, &l1.output, context);
        if !l1.applied_rules.is_empty() {
            println!("L1 applied: {}", l1.applied_rules.join(", "));
        }
        println!();
    }

    if let Some(l2) = l2 {
        print_unified_diff("L1 vs L2", &l1.output, &l2.output, context);
        if !l2.applied_rules.is_empty() {
            println!("L2 applied: {}", l2.applied_rules.join(", "));
        }
    }

    // Suppress the unused-import warning when the function runs in isolation
    // (similar::ChangeTag / TextDiff are consumed inside print_unified_diff).
    let _ = (ChangeTag::Equal, TextDiff::configure());
    Ok(())
}

fn print_unified_diff(header: &str, before: &str, after: &str, context: usize) {
    use similar::{ChangeTag, TextDiff};

    println!("─── {header} ─────────────────────────────");
    let diff = TextDiff::from_lines(before, after);

    // similar's grouped_ops emits hunks centered on changes. Using the
    // requested context size keeps output scannable even for large files.
    for (hunk_idx, group) in diff.grouped_ops(context).iter().enumerate() {
        if hunk_idx > 0 {
            println!("  …");
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let (sign, prefix) = match change.tag() {
                    ChangeTag::Delete => ("-", "- "),
                    ChangeTag::Insert => ("+", "+ "),
                    ChangeTag::Equal => (" ", "  "),
                };
                let old_line = change
                    .old_index()
                    .map(|n| format!("{:>4}", n.saturating_add(1)))
                    .unwrap_or_else(|| "    ".to_string());
                let new_line = change
                    .new_index()
                    .map(|n| format!("{:>4}", n.saturating_add(1)))
                    .unwrap_or_else(|| "    ".to_string());
                // Trim one trailing newline so our println! doesn't double it.
                let text = change.to_string_lossy().trim_end_matches('\n').to_string();
                println!("{old_line} {new_line} {sign}{prefix}{text}");
            }
        }
    }
    println!();
}

fn run_model(action: ModelAction) -> Result<()> {
    match action {
        ModelAction::Setup => run_model_setup(),
        ModelAction::InstallServer => run_install_server(),
        ModelAction::Pull { quant, backend } => run_model_pull(&quant, &backend),
        ModelAction::Test { debug } => run_model_test(debug),
        ModelAction::Bench => run_model_bench(),
        ModelAction::List => run_model_list(),
    }
}

fn run_install_server() -> Result<()> {
    let dest = install_llama_server_binary()?;
    println!();
    println!("llama-server installed at: {}", dest.display());
    println!("Restart the daemon to pick up the new binary:");
    println!("  ntk stop && ntk start");
    Ok(())
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
        println!(
            "Delete it to force a fresh download: rm \"{}\"",
            model_path.display()
        );
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
                let active = if m.contains(&config.model.model_name) {
                    " ← active"
                } else {
                    ""
                };
                println!("  {m}{active}");
            }
        }
    } else {
        // Candle / llama.cpp: just show the configured GGUF path.
        if let Some(p) = &config.model.model_path {
            let status = if p.exists() {
                "✓ found"
            } else {
                "✗ not found"
            };
            println!("Model path: {} [{status}]", p.display());
        } else {
            let default =
                ntk::compressor::layer3_candle::default_model_path(&config.model.quantization)
                    .unwrap_or_default();
            let status = if default.exists() {
                "✓ found"
            } else {
                "✗ not found"
            };
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
    use ntk::gpu;
    use ntk::output::terminal as term;
    use std::io::{self, BufRead, Write};

    println!();
    println!(
        "{}{}  NTK Model Setup Wizard  {}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!(
        "{}══════════════════════════════════════════════════════════════════{}",
        term::dim(),
        term::reset()
    );
    println!();

    // ---- System detection (with spinner) ----
    let sp = term::Spinner::start("Detecting system…");

    let gpu_backend = gpu::detect_best_backend();

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
    println!(
        "{}{}System Info{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!(
        "{}  ────────────────────────────────────{}",
        term::dim(),
        term::reset()
    );

    // CPU line — show model name if available, always show capability tier
    let cpu_cap = gpu::cpu_capability_label();
    let cpu_label = match gpu::cpu_model_name() {
        Some(name) => format!("{name}  {}({cpu_cap}){}", term::dim(), term::reset()),
        None => format!("{}{cpu_cap}{}", term::dim(), term::reset()),
    };
    println!(
        "  {}CPU{}          {}✓{} {}",
        term::dim(),
        term::reset(),
        term::bright_green(),
        term::reset(),
        cpu_label
    );

    // GPU line — show model name if discrete GPU found, otherwise "not detected"
    let gpu_model = gpu::gpu_model_name();
    if let Some(ref name) = gpu_model {
        let vram_label = match &gpu_backend {
            gpu::GpuBackend::CudaNvidia { vram_mb, .. } => {
                format!("  {}({vram_mb} MB VRAM){}", term::dim(), term::reset())
            }
            gpu::GpuBackend::AmdGpu { vram_mb, .. } => {
                format!("  {}({vram_mb} MB VRAM){}", term::dim(), term::reset())
            }
            _ => String::new(),
        };
        println!(
            "  {}GPU{}          {}✓{} {}{}",
            term::dim(),
            term::reset(),
            term::bright_green(),
            term::reset(),
            name,
            vram_label
        );
    } else {
        println!(
            "  {}GPU{}          {}◌ not detected{}",
            term::dim(),
            term::reset(),
            term::dim(),
            term::reset()
        );
    }

    let ollama_color = if ollama_ok {
        term::bright_green()
    } else {
        term::yellow()
    };
    let ollama_icon = if ollama_ok { "✓" } else { "◌" };
    let ollama_status = if ollama_ok { "running" } else { "not running" };
    println!(
        "  {}Ollama{}       {}{} {}{}",
        term::dim(),
        term::reset(),
        ollama_color,
        ollama_icon,
        ollama_status,
        term::reset()
    );

    let llama_color = if llamacpp_ok {
        term::bright_green()
    } else {
        term::yellow()
    };
    let llama_icon = if llamacpp_ok { "✓" } else { "◌" };
    let llama_status = if llamacpp_ok {
        "found in PATH / ~/.ntk/bin"
    } else {
        "not found"
    };
    println!(
        "  {}llama-server{} {}{} {}{}",
        term::dim(),
        term::reset(),
        llama_color,
        llama_icon,
        llama_status,
        term::reset()
    );

    let candle_color = if candle_compiled {
        term::bright_green()
    } else {
        term::yellow()
    };
    let candle_icon = if candle_compiled { "✓" } else { "◌" };
    let candle_status = if candle_compiled {
        "compiled in"
    } else {
        "needs --features candle"
    };
    println!(
        "  {}Candle{}       {}{} {}{}",
        term::dim(),
        term::reset(),
        candle_color,
        candle_icon,
        candle_status,
        term::reset()
    );
    println!();

    // ---- Comparison table (plain text inside cells to keep alignment) ----
    println!(
        "{}{}Backend Comparison{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!("  ┌──────────────────┬─────────────────┬──────────────────────────────────────┐");
    println!("  │ {}Backend{}          │ {}Availability{}    │ {}Summary{}                              │",
        term::bold(), term::reset(), term::bold(), term::reset(), term::bold(), term::reset());
    println!("  ├──────────────────┼─────────────────┼──────────────────────────────────────┤");

    let ollama_av = if ollama_ok {
        "✓ running      "
    } else {
        "◌ needs install"
    };
    println!(
        "  │ {}[1] Ollama{}       │ {} │ External daemon, any model, easiest  │",
        term::bold(),
        term::reset(),
        ollama_av
    );

    let candle_av = if candle_compiled {
        "✓ compiled     "
    } else {
        "◌ needs rebuild"
    };
    println!(
        "  │ {}[2] Candle{}       │ {} │ In-process GGUF, no daemon           │",
        term::bold(),
        term::reset(),
        candle_av
    );

    let llama_av = if llamacpp_ok {
        "✓ found        "
    } else {
        "◌ needs install"
    };
    println!(
        "  │ {}[3] llama.cpp{}    │ {} │ Subprocess, best CPU performance     │",
        term::bold(),
        term::reset(),
        llama_av
    );

    println!("  └──────────────────┴─────────────────┴──────────────────────────────────────┘");
    println!();

    // ---- Pros / cons ----
    println!(
        "{}{}Pros & Cons{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!();

    println!("  {}[1] Ollama{}", term::bold(), term::reset());
    println!(
        "    {}+{} Easiest setup — just `ollama pull phi3:mini`",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}+{} Supports hundreds of models (llama3, mistral, gemma2, qwen2…)",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}+{} Auto GPU/CPU fallback, model management built-in",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}−{} Requires Ollama daemon running alongside NTK (two processes)",
        term::bright_red(),
        term::reset()
    );
    println!(
        "    {}−{} External installation: {}https://ollama.ai{}",
        term::bright_red(),
        term::reset(),
        term::dim(),
        term::reset()
    );
    println!();

    println!(
        "  {}[2] Candle{}  {}(in-process inference){}",
        term::bold(),
        term::reset(),
        term::dim(),
        term::reset()
    );
    println!(
        "    {}+{} Single binary, no external processes",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}+{} Direct CUDA/Metal/CPU access, lowest overhead",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}+{} Works offline once GGUF + tokenizer.json downloaded",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}−{} GGUF + tokenizer.json must be downloaded (~2.2 GB)",
        term::bright_red(),
        term::reset()
    );
    println!(
        "    {}−{} Limited to GGUF-format models (Phi-3, Llama, Mistral…)",
        term::bright_red(),
        term::reset()
    );
    println!();

    println!("  {}[3] llama.cpp{}", term::bold(), term::reset());
    println!(
        "    {}+{} Best CPU performance (AVX2 / AMX optimisations)",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}+{} Excellent CUDA/Metal GPU support",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}+{} Works offline once GGUF downloaded (~2.2 GB)",
        term::bright_green(),
        term::reset()
    );
    println!(
        "    {}−{} Requires llama-server: {}brew install llama.cpp  or GitHub releases{}",
        term::bright_red(),
        term::reset(),
        term::dim(),
        term::reset()
    );
    println!(
        "    {}−{} Extra process to manage (auto-started by NTK daemon)",
        term::bright_red(),
        term::reset()
    );
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

    let rec_name = match recommended {
        2 => "Candle",
        3 => "llama.cpp",
        _ => "Ollama",
    };
    println!(
        "  {}💡  Recommendation: [{}] {}{}",
        term::bright_yellow(),
        recommended,
        rec_name,
        term::reset()
    );
    if recommended == 1 && !ollama_ok {
        println!(
            "  {}    Install Ollama at https://ollama.ai then run: ollama serve{}",
            term::dim(),
            term::reset()
        );
    }
    println!();

    // ---- User choice ----
    print!(
        "{}Choose backend [1/2/3] or Enter for [{}]:{} ",
        term::bold(),
        recommended,
        term::reset()
    );
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
            println!(
                "{}✗  Invalid choice. Run `ntk model setup` again.{}",
                term::bright_red(),
                term::reset()
            );
            return Ok(());
        }
    }

    println!();
    println!(
        "{}✓{}  Configuration saved.",
        term::bright_green(),
        term::reset()
    );
    println!(
        "  {}Restart NTK daemon:{} ntk stop && ntk start",
        term::bold(),
        term::reset()
    );
    Ok(())
}

/// Prompt the user to pick a compute target. Enumerates every detected GPU
/// and offers CPU as the first option. When more than one GPU is present,
/// each GPU is listed separately and the user selects one explicitly.
///
/// Returns `(gpu_layers, gpu_auto_detect, device_id, gpu_vendor)`:
///   * `gpu_layers`: 0 = CPU only, -1 = offload every layer to the chosen GPU
///   * `gpu_auto_detect`: always `false` after the wizard — the user has
///     made an explicit choice
///   * `device_id`: zero-based index within the chosen vendor's enumeration
///     (0 when CPU is picked — unused in that case)
///   * `gpu_vendor`: the vendor of the selected GPU. `None` when CPU is
///     picked or when no discrete GPU exists. Runtime uses this to route
///     inference to the right card on multi-vendor systems instead of
///     silently preferring NVIDIA.
fn setup_gpu_selection() -> Result<(i32, bool, u32, Option<ntk::gpu::GpuVendor>)> {
    use ntk::gpu::{enumerate_gpus, GpuBackend};
    use ntk::output::terminal as term;
    use std::io::{self, BufRead, Write};

    let gpus = enumerate_gpus();

    println!("{}  GPU / Compute Selection{}", term::bold(), term::reset());
    println!(
        "{}  ────────────────────────────────────{}",
        term::dim(),
        term::reset()
    );

    let cpu_label = match ntk::gpu::detect_best_backend() {
        GpuBackend::IntelAmx => "CPU  Intel AMX",
        GpuBackend::Avx512 => "CPU  AVX-512",
        GpuBackend::Avx2 => "CPU  AVX2",
        _ => "CPU  Scalar",
    };

    if gpus.is_empty() {
        println!(
            "  {}Detected:{} {} (no discrete GPU found)",
            term::dim(),
            term::reset(),
            cpu_label
        );
    } else if gpus.len() == 1 {
        println!("  {}Detected:{} {}", term::dim(), term::reset(), gpus[0]);
    } else {
        println!(
            "  {}Detected:{} {} discrete GPUs",
            term::dim(),
            term::reset(),
            gpus.len()
        );
    }
    println!();

    // Option 1 is always CPU; GPUs take options 2..=N+1.
    println!(
        "  {}[1]{}  {:<28}  {}✓ always available{}",
        term::bright_cyan(),
        term::reset(),
        cpu_label,
        term::bright_green(),
        term::reset()
    );

    for (i, gpu) in gpus.iter().enumerate() {
        let num = i.saturating_add(2);
        let vram_label = if gpu.vram_mb > 0 {
            format!("{} MB VRAM", gpu.vram_mb)
        } else {
            "unified memory".to_string()
        };
        println!(
            "  {}[{}]{}  {} {:<20}  {}✓{} {}",
            term::bright_cyan(),
            num,
            term::reset(),
            gpu.vendor.label(),
            gpu.name,
            term::bright_green(),
            term::reset(),
            vram_label
        );
    }

    // Default = first GPU when one is present, otherwise CPU.
    let default_num: usize = if gpus.is_empty() { 1 } else { 2 };

    println!();
    let max_choice = gpus.len().saturating_add(1);
    print!(
        "  {}Choose [1-{}] or Enter for [{}]:{} ",
        term::bright_cyan(),
        max_choice,
        default_num,
        term::reset()
    );
    io::stdout().flush()?;

    let choice_str = io::stdin()
        .lock()
        .lines()
        .next()
        .unwrap_or(Ok(String::new()))?;
    let choice: usize = choice_str.trim().parse().unwrap_or(default_num);

    println!();

    if choice <= 1 {
        return Ok((0, false, 0, None));
    }
    let gpu_idx = choice.saturating_sub(2);
    if let Some(gpu) = gpus.get(gpu_idx) {
        println!(
            "  {}✓{} Using {} (device {})",
            term::bright_green(),
            term::reset(),
            gpu,
            gpu.device_id
        );
        return Ok((-1, false, gpu.device_id, Some(gpu.vendor)));
    }
    // Invalid choice → fall back to CPU rather than crash.
    Ok((0, false, 0, None))
}

fn setup_write_config(provider: &str, existing: &ntk::config::NtkConfig) -> Result<()> {
    use ntk::output::terminal as term;

    let global_path = ntk::config::global_config_path()?;
    let mut config = existing.clone();

    if provider == "ollama" {
        config.model.provider = ntk::config::ModelProvider::Ollama;

        // Detect or install the Ollama runtime (non-fatal on failure —
        // user can install it manually from https://ollama.ai).
        let sp = term::Spinner::start("Configuring Ollama …");
        match ntk::installer::setup_ollama_path() {
            Ok(msg) => sp.finish_ok(&msg),
            Err(e) => sp.finish_warn(&e.to_string()),
        }
    }

    let sp = term::Spinner::start("Saving configuration…");

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
        term::bold(),
        term::reset(),
        term::dim(),
        provider,
        term::reset()
    ));

    println!();
    println!("  {}Next steps for Ollama:{}", term::bold(), term::reset());
    println!(
        "  {}1.{} Install Ollama:     {}https://ollama.ai{}",
        term::bright_cyan(),
        term::reset(),
        term::dim(),
        term::reset()
    );
    println!(
        "  {}2.{} Pull the model:     {}ollama pull phi3:mini{}",
        term::bright_cyan(),
        term::reset(),
        term::dim(),
        term::reset()
    );
    println!(
        "  {}3.{} Start the daemon:   {}ntk start{}",
        term::bright_cyan(),
        term::reset(),
        term::dim(),
        term::reset()
    );
    Ok(())
}

fn setup_candle(existing: &ntk::config::NtkConfig) -> Result<()> {
    use ntk::output::terminal as term;
    use std::io::{self, BufRead, Write};

    if !cfg!(feature = "candle") {
        println!(
            "{}{}Candle is not compiled in the current binary.{}",
            term::bold(),
            term::bright_yellow(),
            term::reset()
        );
        println!();
        println!("  Rebuild NTK with the Candle feature flag:");
        println!(
            "  {}Standard (CPU){}  cargo build --release --features candle",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}NVIDIA GPU{}     cargo build --release --features cuda",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}Apple GPU{}      cargo build --release --features metal",
            term::dim(),
            term::reset()
        );
        println!();
        println!(
            "  {}Then run: ntk model setup{}",
            term::dim(),
            term::reset()
        );
        return Ok(());
    }

    println!(
        "{}{}[2] Candle — In-process inference{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!(
        "{}  ────────────────────────────────────{}",
        term::dim(),
        term::reset()
    );

    let quant = &existing.model.quantization;
    let model_path = ntk::compressor::layer3_candle::default_model_path(quant)?;
    let tokenizer_path = ntk::compressor::layer3_candle::default_tokenizer_path()?;

    let need_model = !model_path.exists();
    let need_tokenizer = !tokenizer_path.exists();

    let model_icon = if need_model {
        format!("{}◌ missing{}", term::yellow(), term::reset())
    } else {
        format!("{}✓ found{}", term::bright_green(), term::reset())
    };
    let tok_icon = if need_tokenizer {
        format!("{}◌ missing{}", term::yellow(), term::reset())
    } else {
        format!("{}✓ found{}", term::bright_green(), term::reset())
    };

    println!(
        "  {}Model{}      {}  {}",
        term::dim(),
        term::reset(),
        model_path.display(),
        model_icon
    );
    println!(
        "  {}Tokenizer{} {}  {}",
        term::dim(),
        term::reset(),
        tokenizer_path.display(),
        tok_icon
    );
    println!();

    if need_model || need_tokenizer {
        let missing_label = match (need_model, need_tokenizer) {
            (true, true) => "model + tokenizer (~2.2 GB)",
            (true, false) => "model (~2.2 GB)",
            _ => "tokenizer (~1 MB)",
        };
        print!(
            "{}Download missing files now?{}  {}[{}]  {}{}[Y/n]:{} ",
            term::bold(),
            term::reset(),
            term::dim(),
            missing_label,
            term::reset(),
            term::bright_cyan(),
            term::reset(),
        );
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
                println!(
                    "\r  {}✓{}  tokenizer.json                                    ",
                    term::bright_green(),
                    term::reset()
                );
            }
            if need_model {
                let quant_upper = quant.to_uppercase();
                let gguf_url = format!(
                    "https://huggingface.co/bartowski/Phi-3-mini-4k-instruct-GGUF/resolve/main/Phi-3-mini-4k-instruct-{quant_upper}.gguf"
                );
                println!(
                    "  {}Downloading model  ({quant_upper}, ~2.2 GB)…{}",
                    term::dim(),
                    term::reset()
                );
                download_file_with_progress(&gguf_url, &model_path)?;
                println!(
                    "\r  {}✓{}  Phi-3-mini-4k-instruct-{quant_upper}.gguf                ",
                    term::bright_green(),
                    term::reset()
                );
            }
            println!();
        }
    } else {
        println!(
            "  {}✓  All files already present — no download needed.{}",
            term::bright_green(),
            term::reset()
        );
        println!();
    }

    let (gpu_layers, gpu_auto_detect, device_id, gpu_vendor) = setup_gpu_selection()?;

    let mut config = existing.clone();
    config.model.provider = ntk::config::ModelProvider::Candle;
    config.model.model_path = Some(model_path);
    config.model.tokenizer_path = Some(tokenizer_path);
    config.model.gpu_layers = gpu_layers;
    config.model.gpu_auto_detect = gpu_auto_detect;
    config.model.cuda_device = device_id;
    config.model.gpu_vendor = gpu_vendor;

    let sp = term::Spinner::start("Saving configuration…");
    let global_path = ntk::config::global_config_path()?;
    let json = serde_json::to_string_pretty(&config)?;
    let tmp = global_path.with_extension("tmp");
    if let Some(parent) = global_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &global_path)?;
    let compute_label = if gpu_layers == 0 { "cpu" } else { "gpu" };
    sp.finish_ok(&format!(
        "{}~/.ntk/config.json{}  {}provider = candle  compute = {}{}",
        term::bold(),
        term::reset(),
        term::dim(),
        compute_label,
        term::reset()
    ));
    Ok(())
}

fn setup_llamacpp(existing: &ntk::config::NtkConfig) -> Result<()> {
    use ntk::output::terminal as term;
    use std::io::{self, BufRead, Write};

    println!(
        "{}{}[3] llama.cpp — Subprocess inference{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!(
        "{}  ────────────────────────────────────{}",
        term::dim(),
        term::reset()
    );
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
                let lib_ext = if cfg!(windows) {
                    "DLLs"
                } else if cfg!(target_os = "macos") {
                    "dylibs"
                } else {
                    "shared libs (.so)"
                };
                println!(
                    "  {}llama-server{}  {}{}{}  {}companion {} missing: {}{}",
                    term::dim(),
                    term::reset(),
                    term::yellow(),
                    p.display(),
                    term::reset(),
                    term::bright_red(),
                    lib_ext,
                    missing_libs.join(", "),
                    term::reset()
                );
                println!();
                print!(
                    "  {}Re-download llama-server with all shared libraries?{}  {}[Y/n]:{} ",
                    term::bold(),
                    term::reset(),
                    term::bright_cyan(),
                    term::reset()
                );
                io::stdout().flush()?;
                let answer = io::stdin()
                    .lock()
                    .lines()
                    .next()
                    .unwrap_or(Ok(String::new()))?;
                if answer.trim().to_lowercase() == "n" {
                    println!(
                        "  {}◌  Keeping existing binary — server may fail to start.{}",
                        term::yellow(),
                        term::reset()
                    );
                    p
                } else {
                    println!();
                    install_llama_server_binary()?
                }
            } else {
                println!(
                    "  {}llama-server{}  {}✓ {}{}",
                    term::dim(),
                    term::reset(),
                    term::bright_green(),
                    p.display(),
                    term::reset()
                );
                p
            }
        }
        Err(_) => {
            println!(
                "  {}llama-server{}  {}◌ not found in PATH or ~/.ntk/bin{}",
                term::dim(),
                term::reset(),
                term::yellow(),
                term::reset()
            );
            println!();
            print!(
                "  {}Download and install llama-server automatically?{}  {}[Y/n]:{} ",
                term::bold(),
                term::reset(),
                term::bright_cyan(),
                term::reset()
            );
            io::stdout().flush()?;
            let answer = io::stdin()
                .lock()
                .lines()
                .next()
                .unwrap_or(Ok(String::new()))?;
            if answer.trim().to_lowercase() == "n" {
                println!();
                println!("  {}Manual install options:{}", term::bold(), term::reset());
                println!(
                    "  {}macOS (Homebrew){}  brew install llama.cpp",
                    term::dim(),
                    term::reset()
                );
                println!(
                    "  {}Linux (apt){}      apt install llama.cpp",
                    term::dim(),
                    term::reset()
                );
                println!(
                    "  {}Releases page{}    {}https://github.com/ggerganov/llama.cpp/releases{}",
                    term::dim(),
                    term::reset(),
                    term::dim(),
                    term::reset()
                );
                println!();
                println!("  Place llama-server in {}~/.ntk/bin/{} or on your PATH, then run `ntk model setup` again.", term::bold(), term::reset());
                return Ok(());
            }
            println!();
            install_llama_server_binary()?
        }
    };
    println!(
        "  {}✓{}  llama-server ready: {}",
        term::bright_green(),
        term::reset(),
        server_binary.display()
    );
    println!();

    let quant = &existing.model.quantization;
    let model_path = ntk::compressor::layer3_candle::default_model_path(quant)?;

    let model_icon = if model_path.exists() {
        format!("{}✓ found{}", term::bright_green(), term::reset())
    } else {
        format!("{}◌ missing{}", term::yellow(), term::reset())
    };
    println!(
        "  {}Model{}  {}  {}",
        term::dim(),
        term::reset(),
        model_path.display(),
        model_icon
    );
    println!();

    if !model_path.exists() {
        let quant_upper = quant.to_uppercase();
        print!(
            "  {}Download GGUF model now?{}  {}({quant_upper}, ~2.2 GB){}  {}[Y/n]:{} ",
            term::bold(),
            term::reset(),
            term::dim(),
            term::reset(),
            term::bright_cyan(),
            term::reset()
        );
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
            println!(
                "  {}Downloading model  ({quant_upper}, ~2.2 GB)…{}",
                term::dim(),
                term::reset()
            );
            download_file_with_progress(&gguf_url, &model_path)?;
            println!(
                "\r  {}✓{}  Phi-3-mini-4k-instruct-{quant_upper}.gguf                ",
                term::bright_green(),
                term::reset()
            );
            println!();
        }
    } else {
        println!(
            "  {}✓  Model already present — no download needed.{}",
            term::bright_green(),
            term::reset()
        );
        println!();
    }

    // Check whether the llama-server binary supports GPU before asking the user
    // to pick one. A CPU-only binary (no Vulkan/CUDA/HIP shared libs) will exit
    // immediately with code 1 when --n-gpu-layers != 0 is passed, so offering
    // GPU options would create a false expectation.
    let server_supports_gpu = ntk::compressor::layer3_llamacpp::binary_supports_gpu(&server_binary);

    let (gpu_layers, gpu_auto_detect, device_id, gpu_vendor) = if server_supports_gpu {
        setup_gpu_selection()?
    } else {
        println!(
            "  {}ℹ  The llama-server binary has no GPU shared libraries (Vulkan / CUDA / HIP).{}",
            term::bright_yellow(),
            term::reset()
        );
        println!(
            "  {}   GPU options are hidden. Replace ~/.ntk/bin/llama-server with a Vulkan{}",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}   or CUDA build from https://github.com/ggerganov/llama.cpp/releases{}",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}   then run `ntk model setup` again to enable GPU selection.{}",
            term::dim(),
            term::reset()
        );
        println!();
        println!(
            "  {}Using CPU inference (always available).{}",
            term::bright_green(),
            term::reset()
        );
        println!();
        (0, false, 0, None)
    };

    let mut config = existing.clone();
    config.model.provider = ntk::config::ModelProvider::LlamaCpp;
    config.model.model_path = Some(model_path);
    config.model.gpu_layers = gpu_layers;
    config.model.gpu_auto_detect = gpu_auto_detect;
    config.model.cuda_device = device_id;
    config.model.gpu_vendor = gpu_vendor;

    let sp = term::Spinner::start("Saving configuration…");
    let global_path = ntk::config::global_config_path()?;
    let json = serde_json::to_string_pretty(&config)?;
    let tmp = global_path.with_extension("tmp");
    if let Some(parent) = global_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &global_path)?;
    let compute_label = if gpu_layers == 0 { "cpu" } else { "gpu" };
    sp.finish_ok(&format!(
        "{}~/.ntk/config.json{}  {}provider = llama_cpp  compute = {}{}",
        term::bold(),
        term::reset(),
        term::dim(),
        compute_label,
        term::reset()
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
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    let bin_dir = home.join(".ntk").join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    let binary_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let dest = bin_dir.join(binary_name);

    // ---- 1. Fetch latest release info from GitHub API ----
    let sp = term::Spinner::start("Fetching latest llama.cpp release from GitHub…");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("ntk-installer/0.1")
        .build()?;

    // The llama.cpp repo was transferred from ggerganov to ggml-org in early
    // 2026; the old path returns a 301 whose Location is another API URL that
    // reqwest follows transparently, but keeping it in sync is cleaner.
    let release: serde_json::Value = client
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("GitHub API request failed: {e}"))?
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("parsing GitHub release JSON: {e}"))?;

    let tag = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    sp.finish_ok(&format!(
        "Latest release: {}{}{}",
        term::bold(),
        tag,
        term::reset()
    ));

    let assets = release
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("no assets in GitHub release"))?;

    // ---- 2. Pick the right asset for this platform/arch ----
    // Read the user's chosen GPU vendor from config so we can pick the best
    // asset automatically: nvidia→cuda, amd→vulkan, apple→macos default,
    // otherwise vulkan (universal) or avx2 (CPU).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cfg = ntk::config::load(&cwd).unwrap_or_default();
    let vendor_hint = cfg.model.gpu_vendor;

    let (asset_name, asset_url) = select_llama_cpp_asset(assets, vendor_hint).ok_or_else(|| {
        anyhow::anyhow!(
            "No suitable llama.cpp binary found for {}/{}\n\
             Download manually from: https://github.com/ggml-org/llama.cpp/releases",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    println!("  {}Asset{}  {}", term::dim(), term::reset(), asset_name);

    // ---- 3. Download the zip into memory ----
    println!("  {}Downloading archive…{}", term::dim(), term::reset());
    let zip_bytes = download_bytes_with_progress(&client, &asset_url).await?;
    println!(
        "\r  {}✓{}  Download complete                                          ",
        term::bright_green(),
        term::reset()
    );

    // ---- 4. Extract llama-server from the zip ----
    let sp = term::Spinner::start("Extracting llama-server from archive…");
    extract_llama_server_from_zip(&zip_bytes, &dest)?;
    sp.finish_ok(&format!(
        "Installed: {}{}{}",
        term::bold(),
        dest.display(),
        term::reset()
    ));

    Ok(dest)
}

/// Pick the best zip asset from the GitHub release assets list for the current OS/arch.
/// Returns `(asset_name, download_url)` or `None` if no match found.
/// Select the best llama.cpp binary for this platform + GPU vendor.
///
/// Preference by vendor (from `config.model.gpu_vendor`):
/// - `nvidia` → cuda-13.1 > cuda-12.4 > vulkan > avx2
/// - `amd` → vulkan > hip-radeon > avx2 (Vulkan is more reliable on older
///   Polaris/Vega cards than ROCm/HIP)
/// - `intel` → sycl > vulkan > avx2
/// - `apple` → macos build (Metal is compiled into every macOS release)
/// - `None` → vulkan > avx2 > plain (safe default)
fn select_llama_cpp_asset(
    assets: &[serde_json::Value],
    vendor: Option<ntk::gpu::GpuVendor>,
) -> Option<(String, String)> {
    let os = std::env::consts::OS; // "linux" | "macos" | "windows"
    let arch = std::env::consts::ARCH; // "x86_64" | "aarch64"

    let os_keywords: &[&str] = match os {
        "linux" => &["linux", "ubuntu"],
        "macos" => &["macos", "osx"],
        "windows" => &["win"],
        _ => return None,
    };
    let arch_keywords: &[&str] = match arch {
        "x86_64" => &["x64"],
        "aarch64" => &["arm64", "aarch64"],
        _ => return None,
    };

    let candidates: Vec<(String, String)> = assets
        .iter()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?;
            let url = a.get("browser_download_url")?.as_str()?;
            if !name.ends_with(".zip") {
                return None;
            }
            let lower = name.to_lowercase();
            if !os_keywords.iter().any(|k| lower.contains(k)) {
                return None;
            }
            if !arch_keywords.iter().any(|k| lower.contains(k)) {
                return None;
            }
            Some((name.to_owned(), url.to_owned()))
        })
        .collect();

    // Tokens in the asset filename that mark a given GPU feature. Ordered
    // within each vendor from best to worst.
    let vendor_pref: &[&str] = match vendor {
        Some(ntk::gpu::GpuVendor::Nvidia) => &["cuda-13.1", "cuda-12.4", "cuda", "vulkan", "avx2"],
        Some(ntk::gpu::GpuVendor::Amd) => &["vulkan", "hip-radeon", "hip", "avx2"],
        // Intel Arc / integrated: SYCL or Vulkan both work; SYCL is
        // Intel-tuned and faster when the OneAPI runtime is present.
        Some(ntk::gpu::GpuVendor::Intel) => &["sycl", "vulkan", "avx2"],
        Some(ntk::gpu::GpuVendor::Apple) => {
            // macOS builds always carry Metal; there are no hip/vulkan/cuda
            // variants on macOS — just pick the first macos+arch match.
            return candidates.into_iter().next();
        }
        None => &["vulkan", "avx2"],
    };

    for token in vendor_pref {
        if let Some(hit) = candidates
            .iter()
            .find(|(n, _)| n.to_lowercase().contains(token))
        {
            return Some(hit.clone());
        }
    }

    // Nothing matched the preferred tokens — last resort: any candidate.
    candidates.into_iter().next()
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
fn extract_llama_server_from_zip(zip_bytes: &[u8], dest: &Path) -> Result<()> {
    use std::io::Read;

    let bin_dir = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("dest has no parent dir"))?;

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| anyhow::anyhow!("opening zip archive: {e}"))?;

    let binary_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
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

        let out_path = if is_binary {
            dest.to_path_buf()
        } else {
            bin_dir.join(&file_part)
        };

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
            std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755)).map_err(
                |e| anyhow::anyhow!("setting permissions on '{}': {e}", out_path.display()),
            )?;
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
async fn download_bytes_with_progress(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    use ntk::output::terminal as term;
    use std::io::Write as _;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("download request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "download returned HTTP {}",
            response.status()
        ));
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
            let pct = bytes
                .len()
                .saturating_mul(100)
                .checked_div(total as usize)
                .unwrap_or(0);
            let mb = bytes.len().saturating_div(1_048_576);
            let total_mb = (total as usize).saturating_div(1_048_576);
            let bar_width = 20usize;
            let filled = pct
                .saturating_mul(bar_width)
                .checked_div(100)
                .unwrap_or(0)
                .min(bar_width);
            let empty = bar_width.saturating_sub(filled);
            let bar_filled = "█".repeat(filled);
            let bar_empty = "░".repeat(empty);
            print!(
                "\r  {}⬇{}  [{}{}{}{}{}]  {}/{} MB  {}%   ",
                term::bright_cyan(),
                term::reset(),
                term::bright_green(),
                bar_filled,
                term::dim(),
                bar_empty,
                term::reset(),
                mb,
                total_mb,
                pct
            );
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
    use ntk::output::terminal as term;
    use std::io::Write as _;

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
                let pct = downloaded
                    .saturating_mul(100)
                    .checked_div(total_bytes)
                    .unwrap_or(0) as usize;
                let mb = downloaded.saturating_div(1_048_576);
                let total_mb = total_bytes.saturating_div(1_048_576);
                let filled = pct
                    .saturating_mul(bar_width)
                    .checked_div(100)
                    .unwrap_or(0)
                    .min(bar_width);
                let empty = bar_width.saturating_sub(filled);
                let bar_filled = "█".repeat(filled);
                let bar_empty = "░".repeat(empty);
                print!(
                    "\r  {}⬇{}  [{}{}{}{}{}]  {}/{} MB  {}%   ",
                    term::bright_cyan(),
                    term::reset(),
                    term::bright_green(),
                    bar_filled,
                    term::dim(),
                    bar_empty,
                    term::reset(),
                    mb,
                    total_mb,
                    pct
                );
                std::io::stdout().flush().ok();
            } else {
                let mb = downloaded / 1_048_576;
                print!(
                    "\r  {}⬇{}  {} MB downloaded…   ",
                    term::bright_cyan(),
                    term::reset(),
                    mb
                );
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

fn run_model_test(debug: bool) -> Result<()> {
    use ntk::compressor::layer3_backend::BackendKind;
    use ntk::compressor::layer3_llamacpp::find_llama_server_binary;
    use ntk::detector::OutputType;
    use ntk::output::terminal as term;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut config = ntk::config::load(&cwd).unwrap_or_default();
    // Use generous timeout for interactive test — first inference after model load can be slow.
    config.model.timeout_ms = 120_000;

    println!(
        "{}Backend:{} {}{}{}",
        term::bold(),
        term::reset(),
        term::bright_cyan(),
        config.model.provider.as_str(),
        term::reset()
    );

    // ── Debug: hardware & config ──────────────────────────────────────────────
    if debug {
        let n_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);
        let gpu_layers = config.model.gpu_layers;
        let gpu_backend = ntk::gpu::resolve_configured_backend(
            gpu_layers,
            config.model.gpu_vendor,
            config.model.cuda_device,
        );
        let use_gpu = gpu_layers != 0;

        // generation threads mirrors layer3_llamacpp::generation_threads()
        let gen_threads = if use_gpu { 4 } else { n_cpus.min(8) };

        println!();
        println!(
            "{}[debug] Hardware / config{}",
            term::bright_cyan(),
            term::reset()
        );
        println!(
            "  logical cpus     : {}",
            if n_cpus == 0 {
                "unknown".to_string()
            } else {
                n_cpus.to_string()
            }
        );
        println!("  gpu backend      : {gpu_backend}");
        println!("  gpu_layers       : {gpu_layers}  (0=CPU, -1=all on GPU)");
        println!("  gen threads      : {gen_threads}  (generation loop)");
        println!("  batch threads    : {n_cpus}  (prefill / prompt processing)");
        println!(
            "  mlock/no-mmap    : {}",
            if use_gpu {
                "off (GPU mode)"
            } else {
                "on  (CPU mode, pins model in RAM)"
            }
        );
        println!(
            "  flash-attn       : {}",
            if use_gpu {
                "on  (GPU mode)"
            } else {
                "off (CPU mode)"
            }
        );

        // CPU model name
        if let Some(cpu_name) = ntk::gpu::cpu_model_name() {
            println!("  cpu model        : {cpu_name}");
        }
        if let Some(gpu_name) = ntk::gpu::gpu_model_name() {
            println!("  gpu model        : {gpu_name}");
        }

        // llama.cpp binary and model file
        if config.model.provider == ntk::config::ModelProvider::LlamaCpp {
            match find_llama_server_binary() {
                Ok(p) => println!("  llama-server bin : {}", p.display()),
                Err(e) => println!("  llama-server bin : NOT FOUND — {e}"),
            }
            if let Some(ref mp) = config.model.model_path {
                let size = std::fs::metadata(mp)
                    .map(|m| {
                        let mb = m.len() / 1_048_576;
                        format!("{mb} MB")
                    })
                    .unwrap_or_else(|_| "file not found".to_string());
                println!("  model path       : {}  ({})", mp.display(), size);
            } else {
                println!("  model path       : not set in config");
            }
        }

        // Quantization
        println!("  quantization     : {}", config.model.quantization);
        println!("  context size     : 4096 tokens");
        println!();
    }

    let backend = BackendKind::from_config(&config)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // If llama.cpp, start the server first (with a generous timeout for model loading).
    if config.model.llama_server_auto_start {
        if let BackendKind::LlamaCpp(_) = &backend {
            let sp = term::Spinner::start("Starting llama-server …");
            let server_start = std::time::Instant::now();
            match rt.block_on(backend.start_if_needed()) {
                Ok(_) => {
                    let startup_ms = server_start.elapsed().as_millis();
                    if debug {
                        sp.finish_ok(&format!(
                            "llama-server ready  {}(startup: {startup_ms}ms){}",
                            term::dim(),
                            term::reset()
                        ));
                    } else {
                        sp.finish_ok("llama-server ready");
                    }
                }
                Err(e) => {
                    sp.finish_err(&format!("llama-server failed: {e}"));
                    return Err(e);
                }
            }
        }
    }

    let test_input = "running 42 tests\ntest result: FAILED. 1 failed; 41 passed; 0 ignored\n\nfailures:\n  test_foo::should_return_42 at src/lib.rs:17\n  left: 0\n  right: 42";
    let prompts_dir = ntk::config::resolve_prompts_dir();

    // ── Debug: show the exact input and prompt ────────────────────────────────
    if debug {
        println!(
            "{}[debug] Test input  ({} chars, {} lines){}",
            term::bright_cyan(),
            test_input.len(),
            test_input.lines().count(),
            term::reset()
        );
        for line in test_input.lines() {
            println!("  │ {line}");
        }
        println!();

        // Show the system prompt that will be sent
        match ntk::compressor::layer3_inference::load_system_prompt(OutputType::Test, &prompts_dir)
        {
            Ok(sp) => {
                let sp_lines: Vec<&str> = sp.lines().collect();
                println!(
                    "{}[debug] System prompt  ({} chars, {} lines){}",
                    term::bright_cyan(),
                    sp.len(),
                    sp_lines.len(),
                    term::reset()
                );
                for line in sp_lines.iter().take(6) {
                    println!("  │ {line}");
                }
                if sp_lines.len() > 6 {
                    println!("  │ … ({} more lines)", sp_lines.len().saturating_sub(6));
                }
                println!();
            }
            Err(e) => println!(
                "{}[debug] Could not load system prompt: {e}{}",
                term::warn_color(),
                term::reset()
            ),
        }
    }

    let sp = term::Spinner::start("Running inference …");
    let start = std::time::Instant::now();
    let result = match rt.block_on(backend.compress(test_input, OutputType::Test, &prompts_dir)) {
        Ok(r) => {
            sp.finish();
            r
        }
        Err(e) => {
            sp.finish_err(&e.to_string());
            return Err(e);
        }
    };
    let elapsed = start.elapsed();

    let compression_pct = 100u64.saturating_sub(
        (result.output_tokens as u64)
            .saturating_mul(100)
            .checked_div(result.input_tokens.max(1) as u64)
            .unwrap_or(100),
    );
    let ratio_pct = result
        .output_tokens
        .saturating_mul(100)
        .checked_div(result.input_tokens.max(1))
        .unwrap_or(0);
    let tok_per_s = if elapsed.as_millis() > 0 {
        result.output_tokens as f64 * 1000.0 / elapsed.as_millis() as f64
    } else {
        0.0
    };

    println!();
    println!(
        "  {}Input  :{} {} tokens",
        term::bold(),
        term::reset(),
        result.input_tokens
    );
    println!(
        "  {}Output :{} {} tokens",
        term::bold(),
        term::reset(),
        result.output_tokens
    );
    println!(
        "  {}Latency:{} {}{:.0}ms{}",
        term::bold(),
        term::reset(),
        term::latency_color(elapsed.as_millis() as u64),
        elapsed.as_millis(),
        term::reset()
    );
    println!(
        "  {}Ratio  :{} {}{}% compression{}  (output is {}% of input)  {}Speed: {tok_per_s:.2} tok/s{}",
        term::bold(),
        term::reset(),
        term::ratio_color(100_usize.saturating_sub(ratio_pct)),
        compression_pct,
        term::reset(),
        ratio_pct,
        term::dim(),
        term::reset()
    );
    println!();

    // ── Debug: performance analysis ───────────────────────────────────────────
    if debug {
        // Derive performance targets from actual hardware rather than using a
        // single desktop-class fixed value.
        //
        // Tiers (CPU mode, Q5_K_M Phi-3 Mini):
        //   GPU            → ≥40 tok/s, <500ms  (conservative for CUDA RTX 3060+)
        //   Mobile / ULP   → ≥5 tok/s,  <5000ms (Core Ultra U, Ryzen U, Snapdragon…)
        //   Desktop        → ≥10 tok/s, <2000ms (i7/i5/Ryzen 5/7 desktop)
        //   High-end / srv → ≥15 tok/s, <1500ms (Xeon, EPYC, Threadripper, i9, Ryzen 9)
        let (expected_min_toks, expected_max_ms, tier_label): (f64, u128, &str) = {
            let use_gpu = config.model.gpu_layers != 0;
            if use_gpu {
                (40.0, 500, "GPU")
            } else {
                let cpu_lower = ntk::gpu::cpu_model_name()
                    .unwrap_or_default()
                    .to_lowercase();
                let last_word = cpu_lower
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .to_string();

                let is_mobile = cpu_lower.contains("ultra")        // Intel Core Ultra (always mobile)
                    || last_word.ends_with('u')                    // "155u", "5500u", "1165g7u"
                    || cpu_lower.contains("snapdragon")
                    || cpu_lower.contains(" m1")
                    || cpu_lower.contains(" m2")
                    || cpu_lower.contains(" m3");

                let is_highend = cpu_lower.contains("xeon")
                    || cpu_lower.contains("epyc")
                    || cpu_lower.contains("threadripper")
                    || cpu_lower.contains("ryzen 9")
                    || cpu_lower.contains("i9-");

                if is_mobile {
                    (5.0, 5000, "mobile/low-power CPU")
                } else if is_highend {
                    (15.0, 1500, "high-end desktop/server CPU")
                } else {
                    (10.0, 2000, "desktop CPU")
                }
            }
        };

        println!(
            "{}[debug] Performance analysis{}",
            term::bright_cyan(),
            term::reset()
        );
        println!(
            "  tok/s            : {:.2}  {}(target ≥{:.0} tok/s for {}){}",
            tok_per_s,
            term::dim(),
            expected_min_toks,
            tier_label,
            term::reset()
        );
        if tok_per_s < expected_min_toks {
            println!(
                "  {}⚠ Slow throughput. Likely causes:{}",
                term::warn_color(),
                term::reset()
            );
            println!("    • Model pages swapped out (mlock not effective or disabled)");
            println!("    • Too many threads causing cache thrashing");
            println!("    • Another process competing for RAM/CPU");
            println!("    • Model quantization too large for available RAM");
        }
        if elapsed.as_millis() > expected_max_ms {
            println!(
                "  {}⚠ High latency: {:.0}ms (target <{}ms on {}){}",
                term::warn_color(),
                elapsed.as_millis(),
                expected_max_ms,
                tier_label,
                term::reset()
            );
        } else {
            println!(
                "  {}✓ Latency within target{}",
                term::bright_green(),
                term::reset()
            );
        }
        if debug {
            println!();
            println!(
                "{}[debug] Output quality check{}",
                term::bright_cyan(),
                term::reset()
            );
        }
    }

    // Quality checks — always computed so the confirmation banner is always shown.
    // The test input intentionally contains 1 failing test; these 4 values must
    // appear in the compressed output for the model to be considered working.
    let out_lower = result.output.to_lowercase();
    let checks: &[(&str, &str)] = &[
        ("1 failed", "failure count (1 failed)"),
        ("41 passed", "pass count (41 passed)"),
        ("test_foo::should_return_42", "test name"),
        ("src/lib.rs", "file location"),
    ];
    let all_passed = checks.iter().all(|(needle, _)| out_lower.contains(needle));

    if debug {
        for (needle, label) in checks {
            let found = out_lower.contains(needle);
            println!(
                "  {} {label:40}  {}{needle}{}",
                if found { "✓" } else { "✗" },
                term::dim(),
                term::reset(),
            );
        }
        println!();
    }

    println!(
        "{}{}Compressed output:{}",
        term::bold(),
        term::bright_cyan(),
        term::reset()
    );
    println!("{}{}{}", term::dim(), "─".repeat(50), term::reset());
    println!("{}", result.output);
    println!("{}{}{}", term::dim(), "─".repeat(50), term::reset());
    println!();

    // Confirmation banner — always visible, not just in debug mode.
    if all_passed {
        println!(
            "  {}{}✓  Output verified — all expected values preserved{}",
            term::bold(),
            term::bright_green(),
            term::reset()
        );
        println!(
            "  {}The model correctly identified: 1 failed · 41 passed · test name · file location{}",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}Note: \"1 failed\" is intentional — the test input contains a deliberate failure{}",
            term::dim(),
            term::reset()
        );
    } else {
        println!(
            "  {}{}✗  Output incomplete — some expected values missing{}",
            term::bold(),
            term::bright_red(),
            term::reset()
        );
        println!(
            "  {}Run with --debug to see which checks failed.{}",
            term::dim(),
            term::reset()
        );
    }

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
        term::bold(),
        term::bright_cyan(),
        term::reset(),
        term::cyan(),
        backend.name(),
        term::reset()
    );
    println!(
        "{}══════════════════════════════════════════════════{}",
        term::dim(),
        term::reset()
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // If llama.cpp, start the server first (no-op for Ollama/Candle).
    if config.model.llama_server_auto_start {
        if let BackendKind::LlamaCpp(_) = &backend {
            let sp = term::Spinner::start("Starting llama-server …");
            match rt.block_on(backend.start_if_needed()) {
                Ok(_) => {
                    sp.finish_ok("llama-server ready");
                    println!();
                }
                Err(e) => {
                    sp.finish_err(&format!("llama-server failed: {e}"));
                    return Err(e);
                }
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
        term::bold(),
        term::white(),
        "payload",
        "min ms",
        "avg ms",
        "max ms",
        "ratio",
        "tok/s",
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
                    if let Some(s) = take_sp() {
                        s.finish();
                    }
                    println!(
                        "{}{label:<28}{}  {}ERROR: {e}{}",
                        term::dim(),
                        term::reset(),
                        term::bright_red(),
                        term::reset()
                    );
                    error_occurred = true;
                    break;
                }
            }
        }

        if latencies_ms.is_empty() || error_occurred {
            continue;
        }

        if let Some(s) = take_sp() {
            s.finish();
        }

        let min = latencies_ms.iter().copied().min().unwrap_or(0);
        let max = latencies_ms.iter().copied().max().unwrap_or(0);
        let avg = latencies_ms
            .iter()
            .copied()
            .sum::<u64>()
            .checked_div(latencies_ms.len() as u64)
            .unwrap_or(0);

        let (ratio_pct, tok_per_s) = if let Some(ref r) = last_result {
            let ratio = r
                .output_tokens
                .saturating_mul(100)
                .checked_div(r.input_tokens.max(1))
                .unwrap_or(0);
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
    let gpu = ntk::gpu::resolve_configured_backend(
        config.model.gpu_layers,
        config.model.gpu_vendor,
        config.model.cuda_device,
    );
    println!(
        "{}GPU backend:{} {}{}{}",
        term::bold(),
        term::reset(),
        term::cyan(),
        gpu,
        term::reset()
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
        let errors_preserved = !case.input.contains("FAILED")
            || l2.output.contains("FAILED")
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
            if l1.lines_removed > 0 || l2.compressed_tokens < original {
                "1+2"
            } else {
                "0"
            },
            l1.lines_removed,
        );

        if !ratio_ok {
            println!(
                "    ✗ ratio {:.1}% < minimum {:.1}%",
                ratio * 100.0,
                case.min_ratio * 100.0
            );
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
            .map_err(|e| anyhow::anyhow!("Layer 3 backend init failed ({provider}): {e}\nRun `ntk model setup` to reconfigure.", provider = config.model.provider.as_str()))?;
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
                        r.output_tokens
                            .saturating_mul(100)
                            .checked_div(r.input_tokens)
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    println!(
                        "  ✓ {:<28}  {}ms  {} → {} tokens  ({ratio}% of input)",
                        case.label, ms, r.input_tokens, r.output_tokens
                    );
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

fn run_bench(runs: usize, with_l3: bool, submit: bool, output: Option<PathBuf>) -> Result<()> {
    use ntk::compressor::{layer1_filter, layer2_tokenizer};
    use std::time::Instant;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = ntk::config::load(&cwd).unwrap_or_default();

    // --submit collects results into a structured report. Human table is
    // still emitted unless --output is set (which implies piping to file,
    // where table formatting would be noise).
    let mut report_payloads: Vec<serde_json::Value> = Vec::new();
    let mut report_l3: Option<serde_json::Value> = None;

    if !submit {
        println!("NTK Compression Benchmark  ({runs} runs per payload)");
        println!("══════════════════════════════════════════════════════════════════");
        println!(
            "{:<28}  {:>7}  {:>8}  {:>8}  {:>8}  {:>7}",
            "payload", "tokens", "min µs", "avg µs", "max µs", "ratio"
        );
        println!("{}", "-".repeat(70));
    }

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

        if submit {
            report_payloads.push(serde_json::json!({
                "label": case.label,
                "tokens_in": original,
                "tokens_out_l2": last_compressed,
                "ratio_pct": ratio_pct,
                "latency_us": {
                    "min": min, "avg": avg, "max": max,
                },
            }));
        } else {
            println!(
                "{:<28}  {:>7}  {:>8}  {:>8}  {:>8}  {:>6}%",
                case.label, original, min, avg, max, ratio_pct
            );
        }
    }

    if with_l3 {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config = ntk::config::load(&cwd).unwrap_or_default();
        let backend = ntk::compressor::layer3_backend::BackendKind::from_config(&config)
            .map_err(|e| anyhow::anyhow!("Layer 3 backend init failed ({provider}): {e}\nRun `ntk model setup` to reconfigure.", provider = config.model.provider.as_str()))?;
        if !submit {
            println!();
            println!(
                "Layer 3 — {} inference  ({runs} runs per payload)",
                backend.name()
            );
            println!(
                "{:<28}  {:>8}  {:>8}  {:>8}  {:>7}",
                "payload", "min ms", "avg ms", "max ms", "ratio"
            );
            println!("{}", "-".repeat(65));
        }

        let prompts_dir = ntk::config::resolve_prompts_dir();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let mut l3_rows: Vec<serde_json::Value> = Vec::new();

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
                        if submit {
                            l3_rows.push(serde_json::json!({
                                "label": case.label,
                                "error": e.to_string(),
                            }));
                        } else {
                            println!("{:<28}  ERROR: {e}", case.label);
                        }
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
            let avg = sum
                .checked_div(l3_latencies.len().max(1) as u64)
                .unwrap_or(0);
            let ratio = last_l3
                .as_ref()
                .map(|r| {
                    if r.input_tokens > 0 {
                        r.output_tokens
                            .saturating_mul(100)
                            .checked_div(r.input_tokens)
                            .unwrap_or(0)
                    } else {
                        0
                    }
                })
                .unwrap_or(0);
            if submit {
                l3_rows.push(serde_json::json!({
                    "label": case.label,
                    "ratio_pct": ratio,
                    "latency_ms": { "min": min, "avg": avg, "max": max },
                }));
            } else {
                println!(
                    "{:<28}  {:>8}  {:>8}  {:>8}  {:>6}%",
                    case.label, min, avg, max, ratio
                );
            }
        }

        if submit {
            report_l3 = Some(serde_json::json!({
                "backend": backend.name(),
                "rows": l3_rows,
            }));
        }
    } else if !submit {
        println!();
        println!("Note: L1+L2 only (pure Rust, no Ollama needed). Add --l3 to include inference.");
    }

    let gpu_backend = ntk::gpu::resolve_configured_backend(
        config.model.gpu_layers,
        config.model.gpu_vendor,
        config.model.cuda_device,
    );

    if submit {
        let report = serde_json::json!({
            "ntk_version": env!("CARGO_PKG_VERSION"),
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu_count": num_cpus_best_effort(),
            "backend": config.model.provider.as_str(),
            "model": config.model.model_name.clone(),
            "quantization": config.model.quantization.clone(),
            "gpu_vendor": config.model.gpu_vendor.as_ref().map(|v| format!("{v:?}")),
            "gpu_backend": format!("{gpu_backend}"),
            "runs_per_payload": runs,
            "payloads": report_payloads,
            "layer3": report_l3,
        });
        let pretty = serde_json::to_string_pretty(&report).context("serializing bench report")?;
        if let Some(path) = output {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&path, &pretty)
                .with_context(|| format!("writing bench report to {}", path.display()))?;
            println!("{}", path.display());
            println!();
            println!("Attach that file to a new issue at:");
            println!("  https://github.com/VALRAW-ALL/ntk/issues/new?template=bench-report.md");
        } else {
            println!("{pretty}");
        }
    } else {
        println!();
        println!("GPU backend: {gpu_backend}");
    }
    Ok(())
}

/// Best-effort CPU-count probe for bench reports. Falls back to 1 when
/// the OS refuses to answer (rare). Kept local to avoid a new dep.
fn num_cpus_best_effort() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

fn run_discover() -> Result<()> {
    use ntk::compressor::layer2_tokenizer;

    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    let transcripts_dir = home.join(".claude").join("transcripts");

    if !transcripts_dir.exists() {
        println!(
            "No Claude transcripts directory found at {}.",
            transcripts_dir.display()
        );
        return Ok(());
    }

    // Collect .jsonl transcript files, newest first.
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&transcripts_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();

    if files.is_empty() {
        println!(
            "No transcript files found in {}.",
            transcripts_dir.display()
        );
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
        let output = val.get("content").and_then(|c| c.as_str()).or_else(|| {
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
    opportunities.sort_by_key(|o| std::cmp::Reverse(o.1));
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
    Ok(format!(
        "http://{}:{}",
        config.daemon.host, config.daemon.port
    ))
}

fn pid_file_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".ntk").join("ntk.pid"))
}

/// Minimal synchronous HTTP GET using reqwest in a blocking context.
fn ureq_get(url: &str) -> Result<String> {
    // The CLI and daemon run as the same user, so the CLI can read the
    // shared-secret token file and authenticate on /metrics, /records,
    // and /state. /health works regardless (it's the open route) so
    // `ntk status` keeps working even without the token file.
    let token = ntk::security::load_or_create_token()
        .ok()
        .unwrap_or_default();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let mut req = client.get(url);
        if !token.is_empty() {
            req = req.header(ntk::security::TOKEN_HEADER, &token);
        }
        let text = req
            .send()
            .await
            .map_err(|e| anyhow!("daemon unreachable — is `ntk start` running? ({e})"))?
            .text()
            .await?;
        Ok(text)
    })
}
