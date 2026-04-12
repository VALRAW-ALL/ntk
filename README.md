# NTK — Neural Token Killer

> Semantic compression proxy for Claude Code. Reduces tool output token count by 60–90% before it reaches the model context — without losing the information that matters.

---

## Table of Contents

- [What it does](#what-it-does)
- [How it works](#how-it-works)
- [Requirements](#requirements)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [GPU Acceleration](#gpu-acceleration)
- [RTK + NTK Coexistence](#rtk--ntk-coexistence)
- [Development](#development)
- [Privacy Policy](#privacy-policy)
- [License](#license)
- [Third-Party Licenses](#third-party-licenses)

---

## What it does

Every time Claude Code runs a Bash command, the output is fed back into the model context. Long outputs from `cargo test`, `tsc`, Docker logs, or `git diff` can consume hundreds or thousands of tokens — slowing down responses and eating through context budgets.

NTK intercepts those outputs via the `PostToolUse` hook, compresses them semantically, and returns a compact version that preserves all errors, warnings, and actionable information. Claude sees less noise, responds faster, and stays focused longer.

**Typical savings:**

| Output type | Example | Savings |
|---|---|---|
| `cargo test` (many passing) | 47 ok + 1 failure | ~85% |
| `tsc` errors | 16 errors in 7 files | ~5% (already dense) |
| Docker logs | Repeated warnings | ~70% |
| Generic command output | Mixed | ~60% |

---

## How it works

NTK runs as a local daemon (`127.0.0.1:8765`) and processes output through up to four layers:

```
Bash tool output
  → PostToolUse hook (ntk-hook.sh / ntk-hook.ps1)
  → HTTP POST /compress  (:8765)
    → Layer 1: Fast Filter       (<1ms)   — ANSI removal, line deduplication, blank-line collapse
    → Layer 2: Tokenizer-Aware   (<5ms)   — BPE path shortening, prefix consolidation (cl100k_base)
    → Layer 3: Local Inference   (opt.)   — Ollama/Phi-3 Mini; only activates above token threshold
    → Layer 4: Context Injection (opt.)   — passes Claude's current intent to the model
  → Compressed output → Claude Code context
```

**Layer 3 activates only when** token count after L1+L2 exceeds `inference_threshold_tokens` (default: 300). Small outputs like `git status` pass through at sub-millisecond latency.

If the daemon is unreachable, the hook falls back gracefully to the original output — NTK never blocks a command.

---

## Requirements

| Requirement | Version |
|---|---|
| Rust | 1.75+ (2021 edition) |
| Cargo | bundled with Rust |
| OS | Windows 10+, macOS 12+, Linux (glibc 2.31+) |

**Optional (for Layer 3 inference):**

| Requirement | Notes |
|---|---|
| [Ollama](https://ollama.com) | Recommended. Manages model download and GPU offloading. |
| NVIDIA GPU (CUDA) | RTX series recommended; tested on RTX 3060+ |
| AMD GPU (ROCm) | Detected via `rocm-smi`; ROCm 5.4+ |
| Apple Silicon (Metal) | M1 and later |

---

## Installation

### Option 1 — From source (recommended while in pre-release)

```bash
# Clone and build
git clone https://github.com/you/ntk
cd ntk
cargo build --release

# Install binary to PATH
cargo install --path .

# Register the PostToolUse hook in Claude Code
# (automatically launches ntk model setup after patching)
ntk init -g
```

### Option 2 — Shell installer (Unix)

```bash
curl -fsSL https://raw.githubusercontent.com/you/ntk/main/scripts/install.sh | bash
```

### Option 3 — PowerShell installer (Windows)

```powershell
irm https://raw.githubusercontent.com/you/ntk/main/scripts/install.ps1 | iex
```

### What `ntk init -g` does

1. Copies the hook script to `~/.ntk/bin/` (`ntk-hook.sh` on Unix, `ntk-hook.ps1` on Windows)
2. Patches `~/.claude/settings.json` to register the `PostToolUse` hook (idempotent — safe to run multiple times)
3. Creates `~/.ntk/config.json` with sensible defaults
4. **Automatically launches `ntk model setup`** — the interactive wizard that detects your CPU/GPU hardware and configures the inference backend

`--hook-only`, `--show`, and `--uninstall` skip the model setup step.

```json
// ~/.claude/settings.json  (added by ntk init -g)
{
  "hooks": {
    "PostToolUse": [{
      "matcher": "Bash",
      "hooks": [{ "type": "command", "command": "~/.ntk/bin/ntk-hook.sh" }]
    }]
  }
}
```

**For OpenCode instead of Claude Code:**

```bash
ntk init -g --opencode
```

**Verify the installation:**

```bash
ntk init --show
```

**Remove the hook:**

```bash
ntk init --uninstall
```

---

## Usage

### Daemon lifecycle

```bash
ntk start           # Start daemon on 127.0.0.1:8765  (opens live TUI dashboard)
                    # If daemon is already running: attaches to the live TUI dashboard
ntk start --gpu     # Start with GPU inference enabled
ntk stop            # Stop the daemon
ntk status          # Show daemon status, loaded model, GPU backend
ntk dashboard       # Combined status + session gain + ASCII bar chart (plain text, non-interactive)
```

**Live dashboard** — `ntk start` opens a full-screen TUI that updates every 500 ms. If the daemon is already running in the background, `ntk start` detects it and **attaches** to the live TUI without restarting the daemon. Press **Ctrl+C** to exit the TUI — the daemon keeps running:

```
┌─────────────────────────────────────────────────────────────┐
│ ██╗  ██╗████████╗██╗  ██╗                                   │
│ ████╗ ██║╚══██╔══╝██║ ██╔╝   Neural Token Killer            │
│ ██╔██╗██║   ██║   █████╔╝    v0.2  •  127.0.0.1:8765       │
│ ██║╚████║   ██║   ██╔═██╗    Uptime: 3m 12s                 │
│ ██║ ╚███║   ██║   ██║  ██╗   Backend: ollama (phi3:mini)    │
│ ╚═╝  ╚══╝   ╚═╝   ╚═╝  ╚═╝                                 │
├─────────────────── SESSION METRICS ─────────────────────────┤
│  Compressions: 47     Tokens In: 84,291  →  Out: 12,048     │
│  Saved: 72,243 tokens  •  Avg ratio: 85%                    │
│                                                             │
│  L1  ████████████████░░░░  38 runs                          │
│  L2  ██████░░░░░░░░░░░░░░   7 runs                          │
│  L3  ██░░░░░░░░░░░░░░░░░░   2 runs                          │
├─────────────────── RECENT COMMANDS ─────────────────────────┤
│  10:14:22  cargo test              1,842  →  312    L2  83% saved │
│  10:14:08  git diff HEAD~1           940  →  188    L2  80% saved │
│  10:13:51  docker logs api         3,200  →  412    L2  87% saved │
└─────────────────────────────────────────────────────────────┘
```

Press **Ctrl+C** in the **attached** TUI to exit the dashboard without stopping the daemon. Press **Ctrl+C** when you started the daemon (first `ntk start`) to stop it gracefully. When stdout is not a TTY (piped or CI), `ntk start` falls back to a single status line.

**Static dashboard** — `ntk dashboard` prints a combined snapshot to stdout and exits immediately (no event loop, always safe to use in scripts or CI):

```
● NTK daemon  running  127.0.0.1:8765  up 3m 22s  backend: ollama (phi3:mini)
  14382 tokens saved across 47 compressions (78% avg ratio)

┌─ NTK · Token Savings ──────────────────────────────────────────────────────┐
│                                                                              │
│  cargo     ████████████████████████████████████████  41823 tok  58%         │
│  git       ████████████████████                      21204 tok  29%         │
│  docker    ████████                                   9101 tok  13%         │
│                                                                              │
│  47 compressions · 72128 tokens saved · 78% avg                             │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Metrics and history

```bash
ntk gain            # Token savings summary (RTK-compatible format)
ntk metrics         # Per-command savings table (requires daemon running)
ntk graph           # ASCII bar chart of savings over time
ntk history         # Last 50 compressed commands with token counts
ntk discover        # Scan latest Claude transcript for missed compression opportunities
```

**Example `ntk gain` output:**

```
NTK: 14382 tokens saved across 47 compressions (78% avg)
```

**Example `ntk history` output:**

```
COMMAND                 TYPE      BEFORE     AFTER  RATIO  LAYER  TIME
--------------------------------------------------------------------------------
cargo test              test        1842       312   83%   L2     2026-04-11 10:00
git diff HEAD~1         diff         940       188   80%   L2     2026-04-11 10:01
docker logs api         log         3200       412   87%   L2     2026-04-11 10:02
```

### Testing compression

```bash
# Test compression on any file without the daemon
ntk test-compress tests/fixtures/cargo_test_output.txt
```

Output:

```
File:             tests/fixtures/cargo_test_output.txt
Original tokens:  512
L1 lines removed: 46
After L2 tokens:  76
Compression:      85.2%

--- Compressed output ---
...
```

### Terminal output

All NTK commands emit colored, animated output when connected to a TTY. Colors are disabled automatically when:

- stdout is redirected to a file or pipe
- The `NO_COLOR` environment variable is set (respects the [no-color.org](https://no-color.org) convention)

Commands that perform inference show a real-time progress animation:

```
⠹ Running inference …           12.3s  [4821 chars]
```

`ntk model bench` shows per-payload progress with elapsed time updating every 250ms while inference runs, followed by a colored results table where compression ratio and latency are color-coded (green → yellow → red by severity).

### Model management (Layer 3)

```bash
# Interactive backend + hardware setup wizard
# Runs automatically after ntk init / ntk init -g.
# Can also be run manually to reconfigure at any time.
ntk model setup

# Download the default model via Ollama
ntk model pull

# Download a specific quantization
ntk model pull --quant q4_k_m   # faster, less RAM
ntk model pull --quant q6_k     # better quality, more RAM

# Test inference latency and output quality
ntk model test

# Test with verbose debug output:
#   hardware config, thread counts, mlock status, system prompt preview,
#   timing breakdown, and performance analysis with CPU-tier-aware targets
#   (mobile/low-power ≥5 tok/s, desktop ≥10, high-end ≥15, GPU ≥40)
ntk model test --debug

# Benchmark CPU vs GPU
ntk model bench
```

### Configuration

```bash
# Show active config
ntk config

# Show config from a specific file
ntk config --file /path/to/.ntk.json
```

---

## Configuration

NTK merges configuration from two sources, in order:

1. `~/.ntk/config.json` — global defaults
2. `.ntk.json` in the current project directory — per-project overrides

**Full reference (`~/.ntk/config.json`):**

```json
{
  "daemon": {
    "port": 8765,
    "host": "127.0.0.1",
    "auto_start": true,
    "log_level": "warn"
  },
  "compression": {
    "enabled": true,
    "layer1_enabled": true,
    "layer2_enabled": true,
    "layer3_enabled": true,
    "inference_threshold_tokens": 300,
    "max_output_tokens": 500,
    "preserve_first_stacktrace": true,
    "preserve_error_counts": true
  },
  "model": {
    "provider": "ollama",
    "model_name": "phi3:mini",
    "quantization": "q5_k_m",
    "ollama_url": "http://localhost:11434",
    "timeout_ms": 2000,
    "fallback_to_layer1_on_timeout": true,
    "gpu_layers": -1,
    "gpu_auto_detect": true
  },
  "metrics": {
    "enabled": true,
    "storage_path": "~/.ntk/metrics.db",
    "history_days": 30
  },
  "exclusions": {
    "commands": ["cat", "echo", "printf"],
    "max_input_chars": 500000
  },
  "telemetry": {
    "enabled": true
  }
}
```

**Key settings:**

| Setting | Default | Description |
|---|---|---|
| `compression.inference_threshold_tokens` | `300` | Layer 3 only activates above this token count |
| `model.fallback_to_layer1_on_timeout` | `true` | Use L1+L2 output if Ollama is slow or unavailable |
| `model.gpu_layers` | `-1` | `-1` = all layers on GPU; `0` = CPU only |
| `exclusions.commands` | `["cat","echo"]` | Commands whose output is never compressed |
| `exclusions.max_input_chars` | `500000` | Hard limit on input size before processing |

**Per-project override example (`.ntk.json` in project root):**

```json
{
  "compression": {
    "inference_threshold_tokens": 100
  },
  "exclusions": {
    "commands": ["make", "just"]
  }
}
```

---

## GPU Acceleration

NTK auto-detects the best inference backend at startup:

```
1. NVIDIA CUDA  (detected via nvidia-smi)
2. AMD ROCm     (detected via rocm-smi)
3. Apple Metal  (aarch64-apple-darwin compile target)
4. Intel AMX    (Xeon 4th Gen / Core Ultra)
5. AVX-512      (is_x86_feature_detected!)
6. AVX2         (most modern x86)
7. CPU Scalar   (fallback)
```

The `ntk model setup` wizard detects your hardware at runtime and presents a GPU/CPU selection step, showing each option with availability status, VRAM, and expected latency. Your choice is saved to `config.model.gpu_layers` (`-1` = GPU, `0` = CPU).

**Performance expectations — Phi-3 Mini 3.8B Q5_K_M (Layer 3 latency p95):**

| Backend | Hardware example | p50 | p95 |
|---|---|---|---|
| CUDA | RTX 3060 | ~50ms | ~80ms |
| CUDA | RTX 5060 Ti | ~30ms | ~50ms |
| AMD ROCm | RX 6800 XT | ~80ms | ~130ms |
| Metal | M2 MacBook Pro | ~80ms | ~150ms |
| Intel AMX | Xeon 4th Gen | ~150ms | ~250ms |
| AVX2 CPU | i7-12700 | ~300ms | ~500ms |
| AVX2 CPU | i5-8250U | ~600ms | ~900ms |

Layer 3 is skipped entirely for outputs below the threshold (default 300 tokens), so most small commands like `git add` or `ls` add zero latency.

**Build options:**

```bash
# Default build — includes Candle (in-process inference, no Ollama needed)
cargo build --release

# CUDA (NVIDIA) — enables GPU offloading for Candle
cargo build --release --features cuda

# Metal (Apple Silicon)
cargo build --release --features metal
```

---

## RTK + NTK Coexistence

NTK is designed to work alongside [RTK (Rust Token Killer)](https://github.com/you/rtk):

- **RTK** runs first, inside the shell command via `rtk <cmd>`. It applies rule-based filtering (regex) synchronously.
- **NTK** runs after, via the `PostToolUse` hook. It applies semantic compression on RTK's already-filtered output.

This double-pass often yields better results than either tool alone:

```
Raw output: 1842 tokens
After RTK:   420 tokens   (rule-based: removed ANSI, grouped repeats)
After NTK:   132 tokens   (semantic: summarized remaining noise)
Combined:    ~93% savings
```

NTK's Layer 1 detects RTK-pre-filtered output (shorter input, no ANSI codes, already contains `[×N]` groupings) and skips redundant processing. Layer 3's threshold often won't trigger on already-filtered output, keeping latency near zero.

```bash
# Both tools active simultaneously — this is the recommended setup
rtk cargo test
# RTK filters in the shell → NTK further compresses via hook
```

---

## Development

### Build

```bash
cargo build           # debug
cargo build --release # release
cargo check           # fast compile check
```

### Test

```bash
# All tests
cargo test

# Individual test suites
cargo test --test layer1_tests
cargo test --test layer2_tests
cargo test --test compression_pipeline_tests
cargo test --test snapshot_tests
cargo test --test quality_regression_tests

# Property-based tests (slow — runs ~256 cases per property)
cargo test --test compression_invariants

# Reduce proptest cases for a faster run
PROPTEST_CASES=32 cargo test --test compression_invariants
```

### Review snapshot changes

After modifying the compression logic, snapshots will fail if the output changes. Review and approve the diffs:

```bash
cargo test --test snapshot_tests   # shows diffs for changed snapshots
cargo insta review                 # interactively approve or reject each change
```

To force-update all snapshots (e.g. after an intentional algorithm improvement):

```bash
INSTA_UPDATE=always cargo test --test snapshot_tests
```

### Benchmarks

```bash
cargo bench                      # run all benchmarks, generate HTML report
cargo bench layer1_1kb           # single benchmark

# Open the HTML report
open target/criterion/report/index.html   # macOS
xdg-open target/criterion/report/index.html  # Linux
```

**Current baseline (debug build, i7-12700):**

| Benchmark | Measured |
|---|---|
| `layer1_1kb` | ~19 µs |
| `layer1_100kb` | < 2 ms |
| `layer2_tokenizer` (1kb) | < 5 ms |
| Full pipeline L1+L2 (1kb) | < 10 ms |

### Linting and security gate

```bash
# Clippy with security lints (required to pass before committing)
cargo clippy -- \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic \
  -W clippy::arithmetic_side_effects \
  -D warnings

# Dependency vulnerability audit
cargo audit

# Format check
cargo fmt --check
```

### Project structure

```
src/
  main.rs                  — CLI (clap) + daemon entry point
  server.rs                — HTTP routes: /compress, /metrics, /health, /state
  config.rs                — Config deserialization + merge + validation
  detector.rs              — Output type detection (test/build/log/diff/generic)
  metrics.rs               — In-memory store + SQLite persistence (sqlx)
  gpu.rs                   — GPU backend detection hierarchy
  installer.rs             — ntk init: idempotent hook + config install
  telemetry.rs             — Anonymous daily telemetry (opt-out)
  compressor/
    layer1_filter.rs       — ANSI strip, dedup, blank-line collapse
    layer2_tokenizer.rs    — tiktoken-rs BPE, path shortening
    layer3_backend.rs      — BackendKind abstraction (Ollama / Candle / LlamaCpp)
    layer3_inference.rs    — Ollama HTTP client + fallback
    layer3_candle.rs       — In-process inference via HuggingFace Candle (CUDA/Metal/CPU)
    layer3_llamacpp.rs     — llama.cpp server client with auto-start
  output/
    terminal.rs            — ANSI colors, TTY detection, Spinner + BenchSpinner
    table.rs               — Metrics tables for stdout
    graph.rs               — ASCII bar charts + sparklines (stdout, non-interactive)
    dashboard.rs           — ratatui TUI: live + attach-mode dashboard (polls /state endpoint)

scripts/
  ntk-hook.sh              — PostToolUse hook (Unix/macOS)
  ntk-hook.ps1             — PostToolUse hook (Windows PowerShell)
  install.sh               — One-line installer (Unix)
  install.ps1              — One-line installer (Windows)

tests/
  unit/                    — Layer 1, Layer 2, detector unit tests
  integration/             — Pipeline, endpoint, CLI, Ollama mock, quality, snapshot tests
  proptest/                — Compression invariants (proptest)
  benchmarks/              — criterion.rs benchmarks
  fixtures/                — Real captured outputs (cargo, tsc, vitest, docker, next.js)
```

---

## Privacy Policy

NTK collects **anonymous, aggregated** usage metrics. No code, file contents, command arguments, paths, or personally identifiable information is ever collected.

### What is collected (once per day, opt-in by default)

| Field | Description |
|---|---|
| `device_hash` | SHA-256(random_salt + machine_id) — not reversible to any personal identifier |
| `ntk_version` | Installed NTK version |
| `os` | Operating system name (`linux`, `macos`, `windows`) |
| `arch` | CPU architecture (`x86_64`, `aarch64`) |
| `compressions_24h` | Number of compressions in the last 24 hours |
| `top_commands` | Most-used command **names only** (e.g. `["cargo", "git"]`) — no arguments, no paths |
| `avg_savings_pct` | Average token savings percentage |
| `layer_pct` | Layer distribution: how often L1, L2, L3 produced the final output |
| `gpu_backend` | Backend used (e.g. `cuda`, `cpu`) |

### What is NOT collected

- Source code or file contents
- Command arguments or flags (e.g. `cargo test --test foo` → only `cargo` is stored)
- File paths, directory names, or project names
- Environment variables or secrets
- IP addresses or network information (telemetry endpoint receives only the JSON payload)
- Any information from the compressed or uncompressed tool outputs

### How the device hash works

A random UUID (salt) is generated once and stored locally in `~/.ntk/.telemetry_salt` with mode `600` (readable only by the file owner on Unix). The salt is combined with a non-personal machine identifier and hashed with SHA-256. The salt is **never sent** — only the hash is. The hash cannot be reversed to identify the machine or the user.

### Opt-out

Telemetry can be disabled in two ways:

```bash
# Environment variable — add to ~/.bashrc or ~/.zshrc for permanent opt-out
export NTK_TELEMETRY_DISABLED=1
```

```json
// ~/.ntk/config.json
{
  "telemetry": { "enabled": false }
}
```

When telemetry is disabled, **no network requests are made** and no payload is constructed.

---

## License

Copyright 2026 Alessandro Mota

Licensed under the **Apache License, Version 2.0** (the "License"). You may not use this software except in compliance with the License.

You may obtain a copy of the License at:

```
http://www.apache.org/licenses/LICENSE-2.0
```

Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific language governing permissions and limitations under the License.

A copy of the full license text is available in the [`LICENSE`](LICENSE) file.

---

## Third-Party Licenses

NTK depends on the following open-source libraries. All are compatible with the Apache-2.0 license.

### Runtime dependencies

| Crate | Version | License | Purpose |
|---|---|---|---|
| [axum](https://github.com/tokio-rs/axum) | 0.7 | MIT | HTTP daemon framework |
| [tokio](https://github.com/tokio-rs/tokio) | 1 | MIT | Async runtime |
| [serde](https://github.com/serde-rs/serde) | 1 | MIT / Apache-2.0 | Serialization |
| [serde_json](https://github.com/serde-rs/json) | 1 | MIT / Apache-2.0 | JSON handling |
| [anyhow](https://github.com/dtolnay/anyhow) | 1 | MIT / Apache-2.0 | Error handling |
| [thiserror](https://github.com/dtolnay/thiserror) | 1 | MIT / Apache-2.0 | Error types |
| [tiktoken-rs](https://github.com/zurawiki/tiktoken-rs) | 0.5 | MIT | BPE tokenizer (cl100k_base) |
| [strip-ansi-escapes](https://github.com/luser/strip-ansi-escapes) | 0.2 | Apache-2.0 | ANSI code removal |
| [sqlx](https://github.com/launchbadge/sqlx) | 0.7 | MIT / Apache-2.0 | Async SQLite persistence |
| [libsqlite3-sys](https://github.com/rusqlite/rusqlite) | 0.27 | MIT | SQLite bundled build |
| [dirs](https://github.com/dirs-dev/dirs-rs) | 5 | MIT / Apache-2.0 | Platform-specific paths |
| [reqwest](https://github.com/seanmonstar/reqwest) | 0.11 | MIT / Apache-2.0 | HTTP client (Ollama, telemetry) |
| [clap](https://github.com/clap-rs/clap) | 4 | MIT / Apache-2.0 | CLI argument parsing |
| [tracing](https://github.com/tokio-rs/tracing) | 0.1 | MIT | Structured logging |
| [tracing-subscriber](https://github.com/tokio-rs/tracing) | 0.3 | MIT | Log output formatting |
| [ratatui](https://github.com/ratatui-org/ratatui) | 0.28 | MIT | ASCII charts (stdout only) |
| [sha2](https://github.com/RustCrypto/hashes) | 0.10 | MIT / Apache-2.0 | SHA-256 for telemetry hash |
| [uuid](https://github.com/uuid-rs/uuid) | 1 | MIT / Apache-2.0 | Random salt generation |
| [url](https://github.com/servo/rust-url) | 2 | MIT / Apache-2.0 | URL validation (Ollama config) |
| [chrono](https://github.com/chronotope/chrono) | 0.4 | MIT / Apache-2.0 | Timestamps in metrics |
| [nix](https://github.com/nix-rust/nix) *(Unix)* | 0.27 | MIT | SIGTERM for `ntk stop` |
| [windows-sys](https://github.com/microsoft/windows-rs) *(Windows)* | 0.52 | MIT / Apache-2.0 | TerminateProcess for `ntk stop` |

### Development / test dependencies

| Crate | Version | License | Purpose |
|---|---|---|---|
| [tempfile](https://github.com/Stebalien/tempfile) | 3 | MIT / Apache-2.0 | Temporary files in tests |
| [wiremock](https://github.com/LukeMathWalker/wiremock-rs) | 0.6 | MIT | Mock Ollama HTTP server |
| [axum-test](https://github.com/JosephLenton/axum-test) | 14 | MIT | Integration test HTTP server |
| [proptest](https://github.com/proptest-rs/proptest) | 1 | MIT / Apache-2.0 | Property-based tests |
| [insta](https://github.com/mitsuhiko/insta) | 1 | Apache-2.0 | Snapshot testing |
| [assert_cmd](https://github.com/assert-rs/assert_cmd) | 2 | MIT / Apache-2.0 | CLI binary tests |
| [criterion](https://github.com/bheisler/criterion.rs) | 0.5 | MIT / Apache-2.0 | Statistical benchmarks |
| [tokio-test](https://github.com/tokio-rs/tokio) | 0.4 | MIT | Async test utilities |
| [predicates](https://github.com/assert-rs/predicates-rs) | 3 | MIT / Apache-2.0 | Test assertions |

To audit the full dependency tree and their licenses, run:

```bash
cargo install cargo-license
cargo license
```

To check for known vulnerabilities in any dependency:

```bash
cargo install cargo-audit
cargo audit
```
