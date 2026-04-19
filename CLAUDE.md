# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status & community

NTK is an **open initiative**. The current maintainer can only cover so
much ground — we actively need collaborators for: stack-trace classifier
coverage (more languages/frameworks), real-hardware benchmarks on GPUs
we don't own (AMD / Apple Silicon / Intel AMX), L3 prompt A/B testing,
hook compatibility with editors other than Claude Code & OpenCode, and
translations of the docs site. Issues and PRs are welcome at
<https://github.com/VALRAW-ALL/ntk>. See **`CONTRIBUTING.md`** for the
concrete starter tasks.

## Implementation Guide

> **Implementando do zero?** Siga `IMPLEMENTATION-SEQUENCE.md` — 26 etapas ordenadas por dependência, com critério de "pronto" em cada uma. Não pule etapas.

## Project-specific rules (`.claude/rules/`)

These rules are loaded automatically when you work on matching files:

| Rule | Triggers on |
|---|---|
| `implementation-gate.md` | **every code change** — security, memory, quality, issue fidelity checklist |
| `clippy-gate.md` | any Rust change — the CI clippy flags that must pass locally |
| `cuda-ci.md` | edits to `.github/workflows/release.yml` with CUDA steps |
| `gpu-vendor.md` | `src/gpu.rs`, `src/config.rs`, `src/main.rs` setup wizard, `layer3_llamacpp.rs` |
| `l1-l2-invariants.md` | any L1/L2 algorithm change — the 5 invariants CI enforces |
| `l1-template-dedup.md` | `group_by_template` changes — blank-line handling |
| `stack-trace-classifier.md` | `is_framework_frame` extensions, new language support |
| `l4-context-injection.md` | `layer4_context.rs`, server handler, hook scripts |

## Project-specific skills (`.claude/skills/`)

| Skill | When to invoke |
|---|---|
| `bench-runner` | Running microbench / full bench / prompt A/B / Claude Code A/B |
| `add-stack-trace-language` | Extending L1's classifier to a new language/framework |

## Skills e Agentes Disponíveis neste Projeto

Skills e agentes do Claude Code configurados para auxiliar na implementação, testes e validação do NTK.

### Skills Auto-Acionadas por Contexto

| Skill | Quando acionar |
|---|---|
| **`write-tests`** | Ao implementar qualquer módulo novo (`layer1_filter.rs`, `layer3_inference.rs`, etc.) — gera suite completa unit + integration |
| **`clean-code`** | Ao finalizar cada etapa do `IMPLEMENTATION-SEQUENCE.md` — verifica naming, funções, SOLID |
| **`architecture-review`** | Antes de iniciar Fases 3, 5 e 8 — valida decisões de design antes de implementar |
| **`dead-code`** | Após Etapa 17 (Layer 3 integrada) e após Etapa 26 (CI) — remove código não usado |
| **`brainstorm`** | Ao encontrar decisão de design não coberta no roadmap — ex: como estruturar detecção RTK output |

### Regras Automáticas por Fase

```
FASE 1-2 (Estrutura + Config):
  → clean-code ao finalizar config.rs (naming em inglês, sem magic numbers)

FASE 2 (Camadas de compressão):
  → write-tests ao finalizar cada layer (layer1, layer2, detector)
  → clean-code antes de commitar cada módulo

FASE 3 (Daemon HTTP):
  → architecture-review antes de definir schema JSON do /compress endpoint
  → write-tests ao finalizar server.rs

FASE 5 (Layer 3 / Inferência):
  → brainstorm se qualidade de compressão nos fixtures for < 70%
  → write-tests para cliente Ollama (mock via wiremock)

FASE 8 (GPU):
  → architecture-review antes de implementar gpu.rs (abstração de backend)

FASE 9 (Testes avançados):
  → write-tests para proptest + snapshots insta
  → dead-code ao finalizar todos os testes
```

### Comandos de Validação por Etapa

Após cada etapa do `IMPLEMENTATION-SEQUENCE.md`, rodar **nesta ordem**:

```bash
# 1. Compila?
cargo check

# 2. Lints de segurança (unwrap, panic, overflow)
cargo clippy -- \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic \
  -W clippy::arithmetic_side_effects \
  -D warnings

# 3. Testes
cargo test

# 4. Formatação
cargo fmt --check

# 5. Auditoria de dependências (se Cargo.toml mudou)
cargo audit
```

Se qualquer um falhar, **não avançar para a próxima etapa**.

### Regra de Segurança (automática)

A regra **`rust-security-audit`** está ativa globalmente e é acionada automaticamente após qualquer implementação de código Rust. Cobre:

- `unwrap()`/`expect()` em produção → panic/DoS
- Validação de tamanho de input antes de alocar
- Path traversal em `ntk test-compress`
- SSRF via `ollama_url` configurável
- Prompt injection na Layer 3
- Escrita atômica em `settings.json`
- Permissões de arquivo (`600`/`700`)
- Integer overflow em operações com dados externos
- `unsafe` blocks obrigatoriamente comentados
- Telemetria: NTK_TELEMETRY_DISABLED verificado antes de coletar

Ver regra completa em: `~/.claude/rules/rust-security-audit.md`

### Agente de Revisão de Arquitetura

Antes de iniciar as fases críticas, usar o agente `Plan` para validar:

```
Fase 3 (Daemon):  Validar schema do endpoint /compress e estrutura do pipeline
Fase 5 (Layer 3): Validar integração Ollama + fallback + threshold logic
Fase 8 (GPU):     Validar abstração GpuBackend e feature flags CUDA/Metal
```

## Project Overview

**NTK (Neural Token Killer)** is a semantic compression proxy daemon written in Rust. It sits between Claude Code's `PostToolUse` hook and the LLM context, compressing command output before it reaches the model. Unlike RTK (rule-based), NTK uses three progressive compression layers with optional local neural inference via Ollama.

## Build & Development Commands

```bash
# Build
cargo build
cargo build --release

# Build with GPU (CUDA)
cargo build --release --features cuda

# Build with Metal (Apple Silicon)
cargo build --release --features metal

# Run tests
cargo test

# Run a single test
cargo test test_name
cargo test layer1::test_remove_ansi_codes

# Run integration tests only
cargo test --test compression_pipeline_tests

# Run property-based tests
cargo test --test proptest_compression

# Run benchmarks (requires Ollama running for inference benchmarks)
cargo bench

# Benchmark specific layer
cargo bench layer1
cargo bench layer2

# Profiling build
cargo build --profile profiling

# CPU flamegraph (requires cargo-flamegraph)
cargo flamegraph --bench compression_bench

# Run quality regression tests
bash tests/quality_check.sh

# Check without building
cargo check
cargo clippy

# Check feature flags
cargo check --features cuda
cargo check --features metal
```

## Installation (ntk init)

```bash
# Recommended — global install with Claude Code hook
ntk init -g

# For OpenCode instead of Claude Code
ntk init -g --opencode

# Non-interactive (CI/CD, scripts)
ntk init -g --auto-patch

# Hook only — skip config.json and docs
ntk init -g --hook-only

# Verify current installation
ntk init --show

# Remove hook from editor settings
ntk init --uninstall
```

`ntk init -g` faz:
1. Copia `ntk-hook.sh` (Unix) ou `ntk-hook.ps1` (Windows) para `~/.ntk/bin/`
2. Patch idempotente em `~/.claude/settings.json` adicionando o hook `PostToolUse`
3. Cria `~/.ntk/config.json` com defaults

## Daemon Commands

```bash
ntk start                    # Start daemon (port 8765 by default)
ntk start --gpu              # Start with GPU inference enabled
ntk stop                     # Stop daemon
ntk status                   # Daemon status + loaded model + GPU info
ntk model pull               # Download phi3:mini via Ollama (~2GB)
ntk model pull --quant q5_k_m  # Download specific quantization
ntk model test               # Test model latency and output quality
ntk model bench              # Benchmark CPU vs GPU inference
ntk test-compress <file>     # Test compression on a captured output file
ntk metrics                  # Session metrics table (stdout, plain text)
ntk graph                    # ASCII bar chart + sparkline via stdout (não-interativo)
ntk gain                     # Token savings summary (RTK-compatible format)
ntk discover                 # Analyze missed RTK/NTK opportunities in session
```

## Architecture

The compression pipeline has 4 layers, applied sequentially:

```
Bash tool output
  → PostToolUse hook (ntk-hook.sh)
  → HTTP POST /compress (daemon on :8765)
    → Layer 1: Fast Filter      (<1ms, always on)  — ANSI removal, line grouping, test failure extraction
    → Layer 2: Tokenizer-Aware  (<5ms, always on)  — tiktoken-rs cl100k_base, BPE path shortening
    → Layer 3: Local Inference  (200-800ms, threshold-based) — Ollama/Phi-3 Mini, type-specific prompts
    → Layer 4: Context Injection (optional) — passes Claude's current intent to the model
  → Compressed output returned to Claude Code
```

**Layer 3 only activates** when output after L1+L2 exceeds `inference_threshold_tokens` (default: 300). This prevents latency overhead on small outputs like `git status`.

## Source Structure

```
src/
  main.rs              — Entry point: CLI parsing (clap) + daemon mode
  server.rs            — Routes: /compress, /metrics, /health
  installer.rs         — ntk init: idempotent patch of editor settings.json + hook copy
  compressor/
    layer1_filter.rs   — ANSI strip, progress bars, diagnostic noise (TS underlines + git index/+++/--- headers), template dedup, stack-trace collapse, prefix factoring, test-failure extraction, blank-line collapse
    layer2_tokenizer.rs — tiktoken-rs integration, BPE path reformatting
    layer3_backend.rs  — BackendKind abstraction: Ollama | Candle | LlamaCpp
    layer3_inference.rs — Ollama HTTP client, type-specific prompts (test/build/log/diff/generic, embedded fallbacks)
    layer3_candle.rs   — In-process inference via HuggingFace Candle (CPU/CUDA/Metal)
    layer3_llamacpp.rs — llama.cpp server client with auto-start
    layer4_context.rs  — Transcript intent extraction + prompt-prefix formatting (Prefix / XmlWrap / Goal / Json)
    spec_loader.rs     — RFC-0001 YAML rule engine (frame-run / line-match / template-dedup / prefix-factor primitives + preserve_errors invariant). Used by `--spec` CLI and the experimental L1.5 daemon stage.
  detector.rs          — Output type detection: test | build | log | diff | generic
  metrics.rs           — In-memory + SQLite (sqlx) persistence
  config.rs            — Deserializes ~/.ntk/config.json + .ntk.json overrides
  gpu.rs               — GPU capability detection (CUDA/Metal/AMX)
  telemetry.rs         — Anonymous usage metrics (opt-out via NTK_TELEMETRY_DISABLED=1)
  output/
    terminal.rs        — ANSI colors, TTY detection (NO_COLOR), Spinner, BenchSpinner
    graph.rs           — ASCII bar charts e sparklines impressos no stdout
    table.rs           — tabelas de histórico formatadas para terminal

scripts/
  ntk-hook.sh          — PostToolUse hook (Unix): reads JSON stdin, POSTs to daemon
  ntk-hook.ps1         — PostToolUse hook (Windows PowerShell)
  install.sh           — System installer: download binary + ntk init -g (Unix)
  install.ps1          — System installer (Windows PowerShell)

tests/
  unit/                — Layer 1, Layer 2, detector unit tests
  integration/         — Full pipeline tests + Ollama mock (wiremock-rs) tests
  proptest/            — Property-based tests: compression invariants
  benchmarks/          — criterion.rs benchmarks + flamegraph integration
  fixtures/            — Real captured outputs: cargo, tsc, vitest, next build, docker logs
```

## Crate Selection Rationale

### Core Crates

| Crate | Purpose | Why |
|---|---|---|
| `axum` | HTTP daemon | Modern async, Tokio-native, good DX |
| `tokio` | Async runtime | Standard for Rust async |
| `tiktoken-rs` | Token counting (cl100k_base) | Same tokenizer as Claude/GPT, Layer 2 |
| `tokenizers` (HuggingFace) | BPE for non-OpenAI models | Layer 2 broader model support |
| `strip-ansi-escapes` | ANSI code removal | 422k/mo downloads, mature, Layer 1 |
| `sqlx` | SQLite async persistence | Compile-time checked queries, async |
| `ratatui` | Terminal output | Tabelas e sparklines ASCII via stdout, sem TUI interativa |
| `serde` + `serde_json` | Config + HTTP JSON | Standard |
| `criterion` | Benchmarks | Statistical regression detection |
| `tracing` + `tracing-subscriber` | Structured logging | Daemon observability |

### Inference Crates (Layer 3)

| Crate | Use Case | Notes |
|---|---|---|
| `ollama-rs` (or direct HTTP) | Primary: Ollama daemon client | Zero model management in NTK binary |
| `candle-core` + `candle-nn` | Optional: in-process inference | No Ollama dependency, CUDA/Metal/CPU |
| `candle-transformers` | Phi-3 / Gemma / Llama via Candle | HuggingFace-maintained |
| `mistral.rs` (via subprocess) | Advanced: PagedAttention, batching | For high-throughput scenarios |
| `llama-gguf` (FFI) | GGUF model loading | If embedding llama.cpp directly |

### Testing Crates

| Crate | Purpose |
|---|---|
| `wiremock` | Mock Ollama HTTP server in tests |
| `axum-test` | Integration test the `/compress` endpoint directly |
| `proptest` | Property-based tests: compression invariants |
| `assert_cmd` | Test the `ntk` CLI binary |
| `insta` | Snapshot testing for compressed outputs |

## Configuration

Global config at `~/.ntk/config.json`. Per-project overrides at `.ntk.json` in project root (merged at runtime). Key settings:

- `compression.inference_threshold_tokens` (default: 300) — Layer 3 activation threshold
- `compression.spec_rules_path` (default: `null`) — RFC-0001 experimental L1.5 stage. Path to a YAML rule file or directory of `*.yaml` rules; composed in filename order. Env var `NTK_SPEC_RULES=<path>` overrides at runtime. Built-in `preserve_errors` invariant guarantees worst-case is a pass-through. Shipped rulesets: `rules/stack_trace/{python,java,go,node,ruby,php,dotnet,kotlin,rust,swift,elixir}.yaml`, `rules/container_log/{docker,kubectl}.yaml`.
- `model.provider` — `"ollama"` | `"candle"` | `"llama_cpp"`
- `model.quantization` — `"q4_k_m"` | `"q5_k_m"` | `"q6_k"` (default: `"q5_k_m"`)
- `model.fallback_to_layer1_on_timeout` — graceful degradation if Ollama is unavailable
- `model.gpu_layers` — number of model layers to offload to GPU (0 = CPU only)
- `exclusions.commands` — commands to pass through uncompressed (e.g., `["cat", "echo"]`)

## Claude Code Integration

The hook intercepts `Bash` tool results via `PostToolUse`. Entry in `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "~/.ntk/bin/ntk-hook.sh" }] }]
  }
}
```

### Hook JSON Schema (PostToolUse stdin)

```json
{
  "session_id": "abc123",
  "transcript_path": "~/.claude/transcripts/abc123.jsonl",
  "cwd": "/project/path",
  "tool_name": "Bash",
  "tool_input": { "command": "cargo test", "description": "Run tests" },
  "tool_response": { "output": "...", "exit_code": 0 }
}
```

### Hook Output (stdout → Claude Code)

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "additionalContext": "NTK compressed: 2341→304 tokens (87%)"
  }
}
```

Exit codes: `0` = success, `1` = non-blocking error (original output used), `2` = block.

The hook skips outputs shorter than 500 chars and falls back to the original output if the daemon is unreachable.

## RTK + NTK Coexistence

NTK is designed to coexist with or replace RTK:

```bash
# RTK still runs first (CLAUDE.md global rule), NTK compresses the result
# RTK: rule-based, synchronous, in the shell command itself
# NTK: semantic, async, via PostToolUse hook after tool completes

# RTK handles: formatting and basic filtering at command level
rtk cargo test

# NTK PostToolUse hook then further compresses the already-filtered output
# → Double compression: RTK cleans up, NTK semantically summarizes

# ntk gain shows combined savings
ntk gain          # NTK session savings
rtk gain          # RTK session savings
```

**When RTK + NTK overlap**: NTK's Layer 1 detects already-filtered RTK output (shorter, no ANSI) and skips redundant processing. Layer 3 threshold may not trigger if RTK already reduced tokens below 300.

## GPU Acceleration

### Detection Hierarchy

NTK auto-detects the best inference backend at startup:

```
1. CUDA GPU (NVIDIA) → candle-cuda or llama.cpp CUDA
2. Metal GPU (Apple Silicon M1+) → candle-metal or Ollama Metal
3. Intel AMX (Xeon 4th Gen / Core Ultra) → llama.cpp AMX
4. AVX-512 → llama.cpp AVX-512
5. AVX2 → llama.cpp AVX2 (default x86 fallback)
6. CPU scalar → minimal config
```

### Performance Expectations (Phi-3 Mini 3.8B, Q5_K_M)

| Backend | Hardware Example | Latency p50 | Latency p95 |
|---|---|---|---|
| CUDA | RTX 3060 | ~50ms | ~80ms |
| CUDA | RTX 5060 Ti | ~30ms | ~50ms |
| Metal | M2 MacBook Pro | ~80ms | ~150ms |
| Intel AMX | Xeon 4th Gen | ~150ms | ~250ms |
| AVX2 CPU | i7-12700 | ~300ms | ~500ms |
| AVX2 CPU | i5-8250U | ~600ms | ~900ms |

### GPU Config (`~/.ntk/config.json`)

```json
{
  "model": {
    "provider": "ollama",
    "gpu_layers": -1,
    "gpu_auto_detect": true,
    "cuda_device": 0,
    "metal_device": 0
  }
}
```

`gpu_layers: -1` = offload all layers (full GPU mode). Set to `0` for CPU-only.

## Performance Targets (MVP Success Criteria)

| Scenario | Target |
|---|---|
| Layer 1+2 latency (100KB) | < 50ms |
| Layer 3 latency p95 (CPU AVX2) | < 1000ms |
| Layer 3 latency p95 (GPU CUDA) | < 100ms |
| Compression ratio — cargo test / tsc | > 70% |
| Compression ratio — verbose logs (L3) | > 90% |
| Error information preservation | 100% of fixtures |
| Graceful fallback (no Ollama/GPU) | L1+L2 only, no crash |

## Profiling Workflow

```bash
# 1. Regression benchmarks (every commit)
cargo bench

# 2. CPU flamegraph when Layer 3 spikes
cargo install flamegraph
cargo flamegraph --bench compression_bench -- --bench bench_full_pipeline_with_inference

# 3. Memory/allocation profiling (Linux)
valgrind --tool=dhat ./target/debug/ntk start

# 4. Layer-specific profiling
cargo bench layer1_100kb    # Should be < 5ms
cargo bench layer2_tokenizer # Should be < 20ms

# Profiling build profile (Cargo.toml)
# [profile.profiling]
# inherits = "release"
# debug = true
# lto = "thin"
# codegen-units = 1
```

## Testing Patterns

### Compression Invariants (proptest)

Key properties to verify:
- Error lines in fixture → error lines present in compressed output
- Token count(compressed) < token count(original)
- Layer 3 not triggered if input < threshold
- Compression is deterministic (same input = same output)

### Ollama Mock (wiremock-rs)

Use `wiremock` to mock the Ollama HTTP API in integration tests without requiring a running Ollama instance:

```rust
// In tests/integration/ollama_mock_tests.rs
let mock_server = MockServer::start().await;
Mock::given(method("POST"))
    .and(path("/api/generate"))
    .respond_with(ResponseTemplate::new(200).set_body_json(...))
    .mount(&mock_server)
    .await;
```

### Snapshot Testing (insta)

Use `insta` for regression testing compressed output format:

```bash
cargo test         # First run: creates snapshots in tests/snapshots/
cargo insta review # Review and approve new/changed snapshots
```

## Telemetry

NTK collects **anonymous, aggregated** usage metrics once per day. Enabled by default to help prioritize development.

### What is collected
- Device hash (SHA-256 with per-user random salt — stored locally, not reversible)
- NTK version, OS, architecture
- Command count (last 24h) and most-used command names (e.g. `"cargo"`, `"git"` — no args, no paths)
- Average token savings percentage
- Layer distribution (L1/L2/L3 %)
- GPU backend used (e.g. `"cuda"`, `"cpu"`)

### What is NOT collected
Source code, file paths, command arguments, secrets, environment variables, or any personally identifiable information.

### Opt-out (any of these)

```bash
# Environment variable (session or ~/.bashrc / ~/.zshrc)
export NTK_TELEMETRY_DISABLED=1

# Or in config file (~/.ntk/config.json)
{
  "telemetry": { "enabled": false }
}
```

### Implementation (`src/telemetry.rs`)

- Sends one HTTP POST per day to telemetry endpoint
- Checks `NTK_TELEMETRY_DISABLED` env var first — if set, skips entirely
- Reads `config.telemetry.enabled` — if false, skips entirely
- Salt stored in `~/.ntk/.telemetry_salt` (generated once, never sent)
- Device hash = `SHA-256(salt + machine_id)` — not reversible to original
- Fire-and-forget: telemetry failure never blocks compression pipeline
- Timeout: 3 seconds max

## Cross-Platform Requirements

NTK must compile and run on **Windows, macOS e Linux** sem modificações. Regras obrigatórias:

- **Sem Unix sockets** — usar TCP `127.0.0.1:8765` (funciona em todos os OS)
- **Paths com `dirs` crate** — nunca hardcodar `/home/user` ou `C:\Users\`. Usar `dirs::home_dir()`, `dirs::config_dir()`
- **Sem `fork()`/`libc` diretamente** — usar `std::process::Command` para subprocessos
- **Caminhos com `std::path::PathBuf`** — nunca concatenar strings com `/` ou `\`
- **`ntk-hook.sh`** — script bash funciona em macOS/Linux. Para Windows: `ntk-hook.ps1` (PowerShell) ou wrapper `.bat` + WSL
- **GPU opcional** — compilar sem `--features cuda/metal` deve produzir binário funcional (CPU-only)
- **SQLite portável** — `sqlx` com SQLite bundled (`sqlx = { features = ["sqlite", "runtime-tokio", "bundled"] }`)
- **Testes CI** — rodar em GitHub Actions matrix: `ubuntu-latest`, `macos-latest`, `windows-latest`

### Hook no Windows

```json
// ~/.claude/settings.json no Windows (WSL ou PowerShell)
{
  "hooks": {
    "PostToolUse": [{
      "matcher": "Bash",
      "hooks": [{ "type": "command", "command": "powershell -File %USERPROFILE%\\.ntk\\bin\\ntk-hook.ps1" }]
    }]
  }
}
```

`ntk install` detecta o OS e configura o hook correto automaticamente.

## Key Design Decisions

- **Rust**: deterministic latency, single binary, no runtime deps — critical for a tool that adds latency to every Bash call
- **Cross-platform first**: TCP over Unix socket, `dirs` crate for paths, dual hook scripts (bash + PowerShell)
- **Phi-3 Mini (3.8B) Q5_K_M**: best quality/speed tradeoff for structured summarization; runs on CPU (~2.8GB RAM), GPU dramatically reduces latency
- **Threshold-based L3**: prevents the 300ms+ inference overhead on small outputs where token savings don't justify it
- **RTK compatibility**: `ntk gain` output matches `rtk gain` format; NTK can coexist with RTK (RTK filters first, NTK summarizes after)
- **No interactive TUI**: all commands print to stdout and exit — no event loops, no alternate screen, no keyboard capture
- **Candle optional backend**: allows in-process CUDA/Metal inference without Ollama daemon dependency
- **sqlx over rusqlite**: async SQLite is required since metrics writes must not block the compression pipeline
- **TCP over Unix socket**: cross-platform (Windows/macOS/Linux), hook script simplicity, negligible latency difference for local use
