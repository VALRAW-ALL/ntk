# NTK — Sequência de Implementação

> Guia sequencial para Claude Code implementar o NTK do zero ao MVP funcional.
> Cada etapa tem pré-requisitos claros, entregáveis e critério de "pronto".
> **Não pule etapas.** Cada etapa compila e testa antes de avançar.

---

## Status de Implementação (atualizado 2026-04-11)

| Fase | Etapas | Status |
|---|---|---|
| FASE 1 — Esqueleto | 1–2 | ✅ Concluída |
| FASE 2 — Compressão Core | 3–6 | ✅ Concluída |
| FASE 3 — Daemon HTTP | 7–10 | ✅ Concluída |
| FASE 4 — CLI | 11–13 | ✅ Concluída |
| FASE 5 — Inferência Local (Layer 3) | 14–17 | ✅ Concluída |
| FASE 6 — Persistência e Métricas | 18 | ✅ Concluída |
| FASE 7 — Output Terminal | 19–20 | ✅ Concluída |
| FASE 8 — GPU e Performance | 21–22 | ✅ Concluída |
| FASE 9 — Testes Avançados | 23–25 | ✅ Concluída |
| FASE 9b — Telemetria | 25b | ✅ Concluída |
| FASE 10 — CI e Publicação | 26 | ⬜ Pendente |

### Módulos implementados além do plano original

- **`src/output/terminal.rs`** — Utilitários de terminal com cores ANSI, spinner braille animado (`Spinner`), spinner com elapsed time real-time (`BenchSpinner`), TTY detection cross-platform, respeito à var `NO_COLOR`
- **`src/compressor/layer3_backend.rs`** — Abstração de backend (`BackendKind`) unificando Ollama, Candle e llama.cpp
- **`src/compressor/layer3_candle.rs`** — Backend in-process via HuggingFace Candle (CUDA/Metal/CPU)
- **`src/compressor/layer3_llamacpp.rs`** — Backend llama.cpp com auto-start do servidor local
- **`src/installer.rs`** — Instalação idempotente em Claude Code, OpenCode; patch atômico de `settings.json`; suporte Windows (`ntk-hook.ps1`) e Unix (`ntk-hook.sh`)
- **`src/telemetry.rs`** — Telemetria anônima, opt-out via `NTK_TELEMETRY_DISABLED`, SHA-256 com salt local

---

## Regras Gerais

- Linguagem: **Rust** (100% do código de produção)
- Após cada etapa: `cargo check` deve passar sem erros
- Após cada etapa com testes: `cargo test` deve passar
- Nenhum `unwrap()` em código de produção — usar `?` ou `anyhow`
- Paths sempre via `std::path::PathBuf` e crate `dirs`
- Sem Unix sockets — apenas TCP `127.0.0.1:8765`

## Gate de Segurança (obrigatório após cada etapa)

Antes de avançar para a próxima etapa, rodar o seguinte bloco completo:

```bash
# SECURITY GATE — rodar após cada etapa implementada

# 1. Sem unwrap/expect/panic em produção
cargo clippy -- \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic \
  -W clippy::arithmetic_side_effects \
  -D warnings

# 2. Testes passam
cargo test

# 3. Auditoria de dependências (obrigatório se Cargo.toml mudou)
cargo audit

# 4. Contar unsafe blocks (deve ser 0 ou cada um deve ter comentário // Safety:)
grep -rn "unsafe" src/ | grep -v "//.*Safety:" | grep -v "#\[cfg(test)\]"
```

**Critério de bloqueio**: qualquer falha nos itens 1-3 bloqueia avanço para próxima etapa.

### Checklist de segurança por módulo

| Módulo | Verificações específicas |
|---|---|
| `config.rs` | ollama_url restrito a localhost; valores numéricos com bounds |
| `layer1_filter.rs` | input size verificado antes de aplicar regex |
| `layer2_tokenizer.rs` | overflow em token count com checked_add |
| `layer3_inference.rs` | timeout enforced; conteúdo só no user turn (não no system prompt) |
| `server.rs` | body size limit no handler /compress; rate no DoS |
| `installer.rs` | escrita atômica em settings.json; backup antes de patch |
| `telemetry.rs` | NTK_TELEMETRY_DISABLED verificado primeiro; sem paths/args no payload |
| `ntk-hook.sh/.ps1` | limite no tamanho do INPUT antes de enviar ao daemon |

---

## FASE 1 — Esqueleto do Projeto

### Etapa 1 — Cargo.toml e estrutura de diretórios

**O que fazer:**
1. Criar `Cargo.toml` com todas as dependências necessárias:
   ```toml
   [package]
   name = "ntk"
   version = "0.1.0"
   edition = "2021"

   [[bin]]
   name = "ntk"
   path = "src/main.rs"

   [dependencies]
   axum = "0.7"
   tokio = { version = "1", features = ["full"] }
   serde = { version = "1", features = ["derive"] }
   serde_json = "1"
   anyhow = "1"
   thiserror = "1"
   tiktoken-rs = "0.5"
   strip-ansi-escapes = "0.2"
   sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio", "bundled"] }
   dirs = "5"
   reqwest = { version = "0.11", features = ["json"] }
   clap = { version = "4", features = ["derive"] }
   tracing = "0.1"
   tracing-subscriber = { version = "0.3", features = ["env-filter"] }
   ratatui = "0.28"

   [dev-dependencies]
   wiremock = "0.6"
   axum-test = "14"
   proptest = "1"
   insta = "1"
   assert_cmd = "2"
   criterion = { version = "0.5", features = ["html_reports"] }
   tokio-test = "0.4"

   # Segurança e criptografia (telemetria)
   # Em [dependencies]:
   sha2 = "0.10"
   uuid = { version = "1", features = ["v4"] }
   url = "2"          # validação de ollama_url

   [features]
   default = []
   cuda = []
   metal = []

   [profile.profiling]
   inherits = "release"
   debug = true
   lto = "thin"
   codegen-units = 1

   [[bench]]
   name = "compression_bench"
   harness = false
   ```

2. Criar estrutura de diretórios vazia com `mod.rs` placeholder em cada módulo:
   ```
   src/
     main.rs
     server.rs
     config.rs
     detector.rs
     metrics.rs
     gpu.rs
     compressor/
       mod.rs
       layer1_filter.rs
       layer2_tokenizer.rs
       layer3_inference.rs
     output/
       mod.rs
       graph.rs
       table.rs
   scripts/
     ntk-hook.sh
     ntk-hook.ps1
     install.sh
   tests/
     unit/
       layer1_tests.rs
       layer2_tests.rs
       detector_tests.rs
     integration/
       compression_pipeline_tests.rs
       ollama_mock_tests.rs
       endpoint_tests.rs
       cli_tests.rs
     proptest/
       compression_invariants.rs
     benchmarks/
       compression_bench.rs
     fixtures/
       cargo_test_output.txt
       tsc_output.txt
       vitest_output.txt
       next_build_output.txt
       docker_logs.txt
   config/
     default_config.json
   system-prompts/
     test.txt
     build.txt
     log.txt
     diff.txt
   ```

**Pronto quando:** `cargo check` passa.

---

### Etapa 2 — Config (`src/config.rs`)

**O que fazer:**
Implementar deserialização do `~/.ntk/config.json` com merge de `.ntk.json` local.

Structs necessárias:
- `NtkConfig` (raiz)
- `DaemonConfig` { port, host, auto_start, log_level }
- `CompressionConfig` { enabled, layer1..3_enabled, inference_threshold_tokens, max_output_tokens, preserve_first_stacktrace, preserve_error_counts }
- `ModelConfig` { provider, model_name, quantization, ollama_url, timeout_ms, fallback_to_layer1_on_timeout, temperature, gpu_layers, gpu_auto_detect }
- `MetricsConfig` { enabled, storage_path, history_days }
- `ExclusionsConfig` { commands: Vec<String>, max_input_chars }
- `DisplayConfig` { show_compression_ratio, show_layer_used, show_backend, color }

Comportamento:
- Se `~/.ntk/config.json` não existe → usar `NtkConfig::default()`
- Se `.ntk.json` existe no cwd → merge (campos presentes sobrescrevem)
- Expandir `~` em paths usando `dirs::home_dir()`

**Pronto quando:** `cargo test config` passa com testes de:
- load default quando arquivo não existe
- merge de config local sobre global
- expansão de `~` em `storage_path`

---

## FASE 2 — Compressão Core (sem modelo)

### Etapa 3 — Layer 1: Fast Filter (`src/compressor/layer1_filter.rs`)

**O que fazer:**
Função principal: `pub fn filter(input: &str) -> String`

Implementar em ordem:
1. **Strip ANSI** — usar `strip_ansi_escapes::strip()`
2. **Detectar RTK output** — se input não tem ANSI e já tem padrão `[×N]`, marcar flag `rtk_pre_filtered`
3. **Remover progress bars** — linhas que correspondem a `\r`, `[====`, `⠋⠙⠹` etc
4. **Agrupar linhas repetidas** — `[×47] cargo:warning=...`
5. **Manter só falhas em test output** — se detectado como test output (via prefixo `FAILED`, `test result:`), descartar linhas de sucesso (`ok`)
6. **Colapsar blank lines consecutivas** — máximo 1 linha em branco seguida

Retornar `Layer1Result { output: String, rtk_pre_filtered: bool, lines_removed: usize }`

**Testes unit em `tests/unit/layer1_tests.rs`:**
- `test_remove_ansi_codes`
- `test_group_repeated_lines`
- `test_keep_only_failures_cargo_test`
- `test_remove_progress_bars`
- `test_collapse_blank_lines`
- `test_detect_rtk_filtered_output`

**Pronto quando:** todos os 6 testes passam + `cargo clippy` sem warnings.

---

### Etapa 4 — Detector de tipo de output (`src/detector.rs`)

**O que fazer:**
Enum e função de detecção:

```rust
pub enum OutputType {
    Test,    // cargo test, vitest, pytest, playwright
    Build,   // cargo build, tsc, next build, eslint
    Log,     // docker logs, journalctl, nginx access log
    Diff,    // git diff, git show, patch
    Generic,
}

pub fn detect(input: &str) -> OutputType
```

Regras de detecção (por ordem de prioridade):
1. **Test**: contém `test result:`, `FAILED`, `passed`, `failed` + padrões vitest/pytest
2. **Build**: contém `error[E`, `TS`, `warning[`, `Compiling`, `Building`
3. **Log**: linhas começam com timestamp ISO ou `[INFO]`/`[ERROR]`/`[WARN]`
4. **Diff**: começa com `diff --git` ou `---`/`+++`
5. **Generic**: fallback

**Testes em `tests/unit/detector_tests.rs`:**
- `test_detects_cargo_test_output`
- `test_detects_tsc_output`
- `test_detects_vitest_output`
- `test_detects_docker_logs`
- `test_detects_git_diff`
- `test_unknown_falls_back_to_generic`

**Pronto quando:** todos os 6 testes passam.

---

### Etapa 5 — Layer 2: Tokenizer-Aware (`src/compressor/layer2_tokenizer.rs`)

**O que fazer:**
Função: `pub fn process(input: &str) -> Layer2Result`

```rust
pub struct Layer2Result {
    pub output: String,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
}
```

Implementar:
1. **Contar tokens** com `tiktoken_rs::cl100k_base()`
2. **Encurtar paths** — `src/components/Button.tsx:10:5` → `Button.tsx:10`
3. **Consolidar prefixos repetidos**:
   ```
   ERROR: src/a.ts
   ERROR: src/b.ts   →   ERROR: a.ts, b.ts, c.ts
   ERROR: src/c.ts
   ```
4. Retornar tokens antes e depois

**Testes em `tests/unit/layer2_tests.rs`:**
- `test_token_count_accuracy` (fixtures conhecidas com contagem esperada)
- `test_path_shortening_reduces_tokens`
- `test_prefix_consolidation`
- `test_no_data_loss_in_reformatting`
- `test_threshold_not_triggered_below_300`

**Pronto quando:** todos os 5 testes passam.

---

### Etapa 6 — Fixtures reais

**O que fazer:**
Popular `tests/fixtures/` com outputs reais capturados:

1. `cargo_test_output.txt` — output de `cargo test` com falhas (~50-200 linhas)
2. `tsc_output.txt` — output de `tsc --noEmit` com erros TypeScript
3. `vitest_output.txt` — output de `vitest run` com falhas
4. `next_build_output.txt` — output de `next build` completo
5. `docker_logs.txt` — logs de container com erros repetidos
6. `cargo_test_rtk_filtered.txt` — versão do cargo_test já processada pelo RTK (sem ANSI, com `[×N]`)

Esses arquivos são dados de teste reais. Criar com conteúdo representativo (pode ser sintético mas realista).

**Pronto quando:** todos os 6 arquivos existem com conteúdo > 20 linhas cada.

---

## FASE 3 — Daemon HTTP

### Etapa 7 — Métricas em memória (`src/metrics.rs`)

**O que fazer:**
Structs e lógica de coleta (sem persistência ainda — SQLite vem depois):

```rust
pub struct CompressionRecord {
    pub command: String,
    pub output_type: OutputType,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub layer_used: u8,          // 1, 2 ou 3
    pub latency_ms: u64,
    pub rtk_pre_filtered: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub struct MetricsStore {
    records: Vec<CompressionRecord>,  // in-memory por ora
}

impl MetricsStore {
    pub fn record(&mut self, r: CompressionRecord)
    pub fn session_summary(&self) -> SessionSummary
    pub fn recent(&self, n: usize) -> &[CompressionRecord]
}
```

**Pronto quando:** `cargo check` passa. Testes na Etapa 13.

---

### Etapa 8 — Servidor HTTP (`src/server.rs` + `src/main.rs`)

**O que fazer:**

Endpoints:
```
POST /compress    — recebe { output: String, context?: String }, retorna { compressed: String, ratio: f32, layer: u8, tokens_before: usize, tokens_after: usize }
GET  /metrics     — retorna SessionSummary em JSON
GET  /health      — retorna { status: "ok", version: String, model: String }
```

`src/main.rs` — apenas inicializa tracing, carrega config, inicia Axum.

Pipeline no `/compress`:
```
input
  → Layer1::filter()
  → Layer2::process()
  → se tokens > threshold → Layer3::compress() [stub por ora — retorna input]
  → retorna resultado
```

**Pronto quando:**
```bash
cargo run &
curl -X POST http://127.0.0.1:8765/compress \
  -H "Content-Type: application/json" \
  -d '{"output": "cargo test output here..."}'
# retorna JSON com campo compressed
curl http://127.0.0.1:8765/health
# retorna {"status":"ok",...}
```

---

### Etapa 9 — Testes de integração do endpoint (`tests/integration/endpoint_tests.rs`)

**O que fazer:**
Usar `axum-test` para testar o endpoint `/compress` diretamente:

```rust
#[tokio::test]
async fn test_compress_endpoint_returns_compressed()
async fn test_compress_short_output_returns_unchanged()
async fn test_health_endpoint()
async fn test_metrics_endpoint_after_compression()
```

**Pronto quando:** todos os 4 testes passam.

---

### Etapa 10 — Testes de pipeline de integração (`tests/integration/compression_pipeline_tests.rs`)

**O que fazer:**
Testes com fixtures reais:

```rust
#[test]
fn test_cargo_test_fixture_compression_ratio()        // ratio >= 0.70
fn test_cargo_test_rtk_pre_filtered_no_redundancy()   // flag rtk_pre_filtered = true
fn test_tsc_errors_preserved()                         // erros presentes no output
fn test_layer3_not_triggered_below_threshold()
fn test_layer3_triggered_above_threshold()             // Layer 3 stub ativado (não comprima ainda)
```

**Pronto quando:** todos os 5 testes passam.

---

## FASE 4 — CLI

### Etapa 11 — CLI com `ntk init` (`src/main.rs` + `src/installer.rs`)

**O que fazer:**

#### 11a — Estrutura de comandos (`src/main.rs` com Clap)

```rust
#[derive(Subcommand)]
enum Command {
    /// Install NTK hook and configuration
    Init(InitArgs),
    /// Start the compression daemon
    Start {
        #[arg(long)] gpu: bool,
        #[arg(long, default_value = "8765")] port: u16,
    },
    /// Stop the daemon
    Stop,
    /// Show daemon status, model and GPU backend
    Status,
    /// Session metrics table
    Metrics,
    /// ASCII bar chart of token savings (stdout, non-interactive)
    Graph,
    /// Token savings summary (RTK-compatible format)
    Gain,
    /// Last N compressed commands
    History {
        #[arg(short, default_value_t = 20)] n: usize,
    },
    /// Open config.json in $EDITOR
    Config,
    /// Test compression on a file
    TestCompress { file: PathBuf },
    /// Model management
    Model(ModelCmd),
    /// Analyze Claude Code session for missed NTK opportunities
    Discover,
}

#[derive(Args)]
struct InitArgs {
    /// Install globally in ~/.ntk (recommended)
    #[arg(short = 'g', long)]
    global: bool,

    /// Target editor/runner
    #[arg(long, value_enum, default_value = "claude-code")]
    editor: EditorTarget,

    /// Non-interactive mode for CI/CD (no prompts)
    #[arg(long)]
    auto_patch: bool,

    /// Install hook only — skip config and docs
    #[arg(long)]
    hook_only: bool,

    /// Show current installation status and exit
    #[arg(long)]
    show: bool,

    /// Remove NTK hook and config
    #[arg(long)]
    uninstall: bool,
}

#[derive(ValueEnum, Clone)]
enum EditorTarget {
    ClaudeCode,   // ~/.claude/settings.json
    OpenCode,     // ~/.opencode/config.json (ou equivalente)
}
```

#### 11b — Módulo de instalação (`src/installer.rs`)

Responsabilidade: detectar OS, localizar arquivos de config do editor, instalar/remover hook.

```rust
pub struct Installer {
    global: bool,
    editor: EditorTarget,
    auto_patch: bool,
    hook_only: bool,
}

impl Installer {
    pub fn run(&self) -> anyhow::Result<()>
    pub fn show_status(&self) -> anyhow::Result<()>
    pub fn uninstall(&self) -> anyhow::Result<()>
}
```

Lógica de `run()`:

```
1. Detectar OS → selecionar hook script (ntk-hook.sh ou ntk-hook.ps1)
2. Criar ~/.ntk/bin/ se não existe
3. Copiar hook script para ~/.ntk/bin/
4. chmod +x em Unix
5. Localizar settings.json do editor:
     ClaudeCode: ~/.claude/settings.json
     OpenCode:   ~/.opencode/settings.json (ou config equivalente)
6. Fazer patch no settings.json (inserir bloco PostToolUse)
     - se já existe entrada NTK → skip (idempotente)
     - se auto_patch=false → mostrar diff e pedir confirmação
7. Se hook_only=false:
     a. Criar ~/.ntk/config.json com defaults
     b. (opcional, se auto_patch=false) perguntar se quer baixar modelo
8. Imprimir resumo do que foi feito
```

Lógica de `show_status()`:
```
NTK Installation Status
-----------------------
Binary:      /usr/local/bin/ntk (v0.1.0)       ✓
Hook script: ~/.ntk/bin/ntk-hook.sh             ✓
Config:      ~/.ntk/config.json                 ✓
Editor hook: ~/.claude/settings.json            ✓  (Claude Code)
Daemon:      running on :8765                   ✓
Model:       phi3:mini Q5_K_M via Ollama        ✓
GPU:         CUDA (RTX 3060)                    ✓
```

Lógica de `uninstall()`:
```
1. Remover bloco NTK de settings.json do editor
2. Remover ~/.ntk/bin/ntk-hook.*
3. (preservar) ~/.ntk/config.json e ~/.ntk/metrics.db  ← dados do usuário
4. Imprimir confirmação
```

#### 11c — Comportamento por combinação de flags

| Comando | Comportamento |
|---|---|
| `ntk init` | Instala localmente no projeto (`.ntk.json` + hook local) |
| `ntk init -g` | Instala globalmente em `~/.ntk/` + hook em `~/.claude/settings.json` |
| `ntk init -g --opencode` | Mesmo, mas hook em `~/.opencode/settings.json` |
| `ntk init -g --auto-patch` | Sem prompts — usa defaults, não pergunta nada |
| `ntk init -g --hook-only` | Só instala hook, não cria `~/.ntk/config.json` |
| `ntk init --show` | Mostra status da instalação e sai (não instala nada) |
| `ntk init --uninstall` | Remove hook do settings.json do editor |

#### 11d — Patch do settings.json (idempotente)

O patch é **idempotente**: rodar `ntk init -g` duas vezes não duplica o hook.

Estrutura do patch para Claude Code:
```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{
          "type": "command",
          "command": "~/.ntk/bin/ntk-hook.sh"
        }]
      }
    ]
  }
}
```

Para Windows, o comando é `powershell -NoProfile -File ~/.ntk/bin/ntk-hook.ps1`.

Detecção de OS no Rust:
```rust
#[cfg(target_os = "windows")]
fn hook_command() -> String { "powershell -NoProfile -File ~/.ntk/bin/ntk-hook.ps1".into() }

#[cfg(not(target_os = "windows"))]
fn hook_command() -> String { "~/.ntk/bin/ntk-hook.sh".into() }
```

#### 11e — Daemon: start/stop/status

1. `ntk start` — spawn daemon como processo filho detached, salva PID em `~/.ntk/ntk.pid`
2. `ntk stop` — lê PID, envia:
   - Unix: `kill(pid, SIGTERM)` via `nix` crate
   - Windows: `TerminateProcess` via `winapi` ou `windows-sys` crate
3. `ntk status` — GET `/health`, imprime resposta formatada

#### 11f — Outros comandos MVP

```rust
// ntk gain — RTK-compatible output
fn cmd_gain(summary: &SessionSummary) {
    println!("NTK — Token savings this session");
    println!("Commands compressed: {}", summary.total_commands);
    println!("Tokens saved:        {} ({:.0}% avg reduction)", summary.tokens_saved, summary.avg_ratio * 100.0);
    println!("Layer distribution:  L1 {}%  L2 {}%  L3 {}%", ...);
}

// ntk test-compress <file>
fn cmd_test_compress(file: &Path) {
    // lê arquivo, POST /compress, imprime resultado com ratio
}

// ntk metrics — tabela detalhada
fn cmd_metrics(records: &[CompressionRecord]) {
    // tabela ASCII: CMD | TYPE | BEFORE | AFTER | RATIO | LAYER | RTK
}
```

Stubs para implementar depois: `graph`, `history`, `discover`, `model`

**Pronto quando:**
```bash
ntk init -g          # instala hook + config, imprime resumo
ntk init --show      # mostra status completo
ntk init --uninstall # remove hook
ntk start            # daemon inicia em background
ntk status           # mostra daemon ok
ntk stop             # daemon para
ntk gain             # imprime savings no formato RTK
```

---

### Etapa 12 — Scripts de hook e instaladores do sistema

**O que fazer:**

`scripts/ntk-hook.sh` (bash — macOS/Linux):
```bash
#!/bin/bash
INPUT=$(cat)
OUTPUT=$(echo "$INPUT" | jq -r '.tool_response.output // .output // empty')
[ -z "$OUTPUT" ] || [ ${#OUTPUT} -lt 500 ] && echo "$INPUT" && exit 0
COMPRESSED=$(curl -sf -X POST http://127.0.0.1:8765/compress \
  -H "Content-Type: application/json" \
  -d "{\"output\": $(echo "$OUTPUT" | jq -Rs .)}" \
  | jq -r '.compressed // empty')
[ -n "$COMPRESSED" ] && echo "$INPUT" | jq --arg c "$COMPRESSED" '.tool_response.output = $c' || echo "$INPUT"
```

`scripts/ntk-hook.ps1` (PowerShell — Windows):
```powershell
$input_data = $input | ConvertFrom-Json
$output = $input_data.tool_response.output
if (-not $output -or $output.Length -lt 500) { $input | Write-Output; exit 0 }
$body = @{ output = $output } | ConvertTo-Json
$result = Invoke-RestMethod -Uri "http://127.0.0.1:8765/compress" -Method POST -Body $body -ContentType "application/json" -ErrorAction SilentlyContinue
if ($result.compressed) {
    $input_data.tool_response.output = $result.compressed
    $input_data | ConvertTo-Json | Write-Output
} else { $input | Write-Output }
```

`ntk install` copia o script correto para `~/.ntk/bin/` e adiciona ao `~/.claude/settings.json`.

#### Instaladores do sistema (distribuição)

`install.sh` (macOS/Linux — one-liner):
```bash
#!/bin/sh
# curl -sSf https://raw.githubusercontent.com/user/ntk/main/install.sh | sh
set -e
LATEST=$(curl -sSf https://api.github.com/repos/user/ntk/releases/latest | grep tag_name | cut -d'"' -f4)
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
[ "$ARCH" = "x86_64" ] && ARCH="x86_64"
[ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ] && ARCH="aarch64"
URL="https://github.com/user/ntk/releases/download/${LATEST}/ntk-${OS}-${ARCH}"
curl -sSfL "$URL" -o /tmp/ntk && chmod +x /tmp/ntk
sudo mv /tmp/ntk /usr/local/bin/ntk
echo "NTK installed. Run: ntk init -g"
```

`install.ps1` (Windows — one-liner PowerShell):
```powershell
# irm https://raw.githubusercontent.com/user/ntk/main/install.ps1 | iex
$latest = (Invoke-RestMethod "https://api.github.com/repos/user/ntk/releases/latest").tag_name
$url = "https://github.com/user/ntk/releases/download/$latest/ntk-windows-x86_64.exe"
$dest = "$env:LOCALAPPDATA\ntk\ntk.exe"
New-Item -ItemType Directory -Force -Path "$env:LOCALAPPDATA\ntk" | Out-Null
Invoke-WebRequest $url -OutFile $dest
# Adiciona ao PATH se não estiver
$path = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($path -notlike "*ntk*") {
    [Environment]::SetEnvironmentVariable("PATH", "$path;$env:LOCALAPPDATA\ntk", "User")
}
Write-Host "NTK installed. Run: ntk init -g"
```

Métodos de instalação suportados:
```bash
# macOS/Linux — script
curl -sSf https://...install.sh | sh

# macOS — Homebrew (futuro)
brew install ntk

# Windows — script PowerShell
irm https://...install.ps1 | iex

# Windows — Winget (futuro)
winget install ntk

# Qualquer plataforma — Cargo
cargo install ntk
```

Estrutura de release no GitHub Actions:
```yaml
# .github/workflows/release.yml
strategy:
  matrix:
    include:
      - os: ubuntu-latest,  target: x86_64-unknown-linux-musl
      - os: macos-latest,   target: x86_64-apple-darwin
      - os: macos-latest,   target: aarch64-apple-darwin
      - os: windows-latest, target: x86_64-pc-windows-msvc
```

Adicionar ao `Cargo.toml`:
```toml
[dependencies]
# Para signal handling cross-platform
signal-hook = { version = "0.3", optional = true }

[target.'cfg(unix)'.dependencies]
nix = { version = "0.27", features = ["signal"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.52", features = ["Win32_System_Threading"] }
```

**Pronto quando:** hook funciona end-to-end com Claude Code local E `ntk init -g` completa sem erros nos 3 OS.

---

### Etapa 13 — Testes CLI (`tests/integration/cli_tests.rs`)

**O que fazer:**
Usar `assert_cmd`:

```rust
#[test]
fn test_ntk_status_without_daemon()     // deve retornar erro legível
fn test_ntk_install_creates_hook()      // verifica settings.json
fn test_ntk_uninstall_removes_hook()
fn test_ntk_test_compress_file()        // comprime fixture e imprime ratio
fn test_ntk_gain_format_rtk_compatible()
```

**Pronto quando:** todos os 5 testes passam.

---

## FASE 5 — Inferência Local (Layer 3)

### Etapa 14 — System prompts

**O que fazer:**
Criar os 4 arquivos em `system-prompts/`:

`test.txt`:
```
You are a test output compressor. Given raw test runner output, extract ONLY:
1. Number of tests: X passed, Y failed, Z skipped
2. For each FAILED test: test name + exact error message + file:line
3. Total duration
Discard: passing test names, progress bars, coverage tables, warnings.
Output in compact format. No prose.
```

`build.txt`:
```
You are a build output compressor. Extract ONLY:
1. Build result: success/failed
2. For each ERROR: file:line + error code + message (1 line)
3. Warning count only (not individual warnings unless < 3)
4. Build duration if present
Discard: info messages, progress, module counts, asset sizes.
```

`log.txt`:
```
You are a log compressor. Extract:
1. ERROR and CRITICAL lines (all, with timestamp and count if repeated)
2. WARN lines grouped: "[xN] message" if same message repeats
3. Any exception/stack trace (first occurrence only)
4. Summary: X errors, Y warnings in N lines
Discard: INFO, DEBUG, TRACE lines unless in the 3 lines before an error.
```

`diff.txt`:
```
You are a diff compressor. Extract:
1. Files changed (list)
2. For each file: summary of what changed in 1 line
3. Total: X files, +Y -Z lines
Discard: unchanged context lines, hunk headers.
```

**Pronto quando:** 4 arquivos existem.

---

### Etapa 15 — Layer 3: Cliente Ollama (`src/compressor/layer3_inference.rs`)

**O que fazer:**
Cliente HTTP para a API do Ollama:

```rust
pub struct OllamaClient {
    base_url: String,
    timeout: Duration,
}

impl OllamaClient {
    pub async fn compress(
        &self,
        input: &str,
        output_type: OutputType,
        prompts_dir: &Path,
    ) -> anyhow::Result<String>
}
```

Implementar:
1. Carrega prompt do tipo em `system-prompts/{type}.txt`
2. POST `{ollama_url}/api/generate` com `{ model, prompt, system, stream: false }`
3. Timeout via `model.timeout_ms` do config
4. Se timeout ou erro de conexão → retornar `Err()` (caller faz fallback)

**Pronto quando:** `cargo check` passa.

---

### Etapa 16 — Mock Ollama e testes Layer 3 (`tests/integration/ollama_mock_tests.rs`)

**O que fazer:**
Usar `wiremock` para mockar o servidor Ollama:

```rust
#[tokio::test]
async fn test_inference_request_format()       // verifica body enviado ao Ollama
async fn test_fallback_on_ollama_timeout()     // timeout → retorna Layer1+2
async fn test_fallback_on_ollama_unavailable() // sem servidor → retorna Layer1+2
async fn test_correct_prompt_per_type()        // test→test.txt, build→build.txt
```

**Pronto quando:** todos os 4 testes passam sem Ollama real instalado.

---

### Etapa 17 — Integrar Layer 3 no pipeline

**O que fazer:**
Em `src/server.rs` (`/compress`), substituir o stub da Layer 3:

```rust
if layer2_result.compressed_tokens > config.compression.inference_threshold_tokens {
    match ollama_client.compress(&layer2_result.output, output_type, &prompts_dir).await {
        Ok(compressed) => use_compressed(compressed, layer = 3),
        Err(_) => use_fallback(layer2_result.output, layer = 2),  // fallback gracioso
    }
}
```

**Pronto quando:**
```bash
ntk start
cat tests/fixtures/cargo_test_output.txt | ntk test-compress /dev/stdin
# mostra ratio de compressão com Layer 3
```

---

## FASE 6 — Persistência e Métricas

### Etapa 18 — SQLite com sqlx (`src/metrics.rs`)

**O que fazer:**
Adicionar persistência ao `MetricsStore`:

Schema:
```sql
CREATE TABLE IF NOT EXISTS compression_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command TEXT,
    output_type TEXT,
    original_tokens INTEGER,
    compressed_tokens INTEGER,
    layer_used INTEGER,
    latency_ms INTEGER,
    rtk_pre_filtered BOOLEAN,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

Métodos:
```rust
pub async fn init_db(path: &Path) -> anyhow::Result<SqlitePool>
pub async fn persist(&self, record: &CompressionRecord) -> anyhow::Result<()>
pub async fn history(&self, n: usize) -> anyhow::Result<Vec<CompressionRecord>>
pub async fn weekly_summary(&self) -> anyhow::Result<WeeklySummary>
```

**Pronto quando:** `ntk metrics` mostra dados persistidos após restart do daemon.

---

## FASE 7 — Output Terminal

### Etapa 19 — Formatação de output (`src/output/`) ✅

**O que fazer:**
Funções que imprimem no stdout e encerram (não-interativo):

**`src/output/terminal.rs`** ✅ (módulo adicionado além do plano original):
```rust
// Cores ANSI com TTY detection + NO_COLOR
pub fn color_enabled() -> bool     // OnceLock, detecta TTY e NO_COLOR
pub fn ratio_color(pct: usize) -> &'static str   // ≤40% verde, ≤80% amarelo, >80% vermelho
pub fn latency_color(ms: u64) -> &'static str    // ≤1s verde, ≤5s amarelo, >5s vermelho

// Spinner braille animado (thread background, 80ms/frame)
pub struct Spinner { ... }
impl Spinner {
    pub fn start(label: &str) -> Self
    pub fn finish(self)
    pub fn finish_ok(self, msg: &str)   // ✓ verde
    pub fn finish_err(self, msg: &str)  // ✗ vermelho
}

// Spinner com elapsed time real-time (250ms/frame)
pub struct BenchSpinner { ... }
impl BenchSpinner {
    pub fn start(label: &str, input_chars: usize) -> Self
    // Exibe: ⠋ label<28  elapsed.1s  [N chars]
    pub fn finish(self)
}
```

`src/output/table.rs`:
```rust
pub fn print_metrics_table(records: &[CompressionRecord])
pub fn print_session_summary(summary: &SessionSummary)
pub fn print_gain_rtk_compat(summary: &SessionSummary)  // formato idêntico ao rtk gain
```

`src/output/graph.rs`:
```rust
pub fn print_bar_chart(records: &[CompressionRecord])   // barras ASCII por comando
pub fn print_sparkline_weekly(summary: &WeeklySummary)   // sparkline dos últimos 7 dias
pub fn print_layer_distribution(summary: &SessionSummary)
```

Usar `ratatui` apenas para renderizar widgets em buffer e depois imprimir como string — sem `CrosstermBackend` em modo alternado.

**Pronto quando:**
```bash
ntk graph   # imprime ASCII e retorna ao prompt
ntk metrics # imprime tabela e retorna ao prompt
ntk gain    # imprime resumo RTK-compatível
```

---

### Etapa 20 — `ntk history` e `ntk discover`

**O que fazer:**

`ntk history` — lê últimos N registros do SQLite, imprime tabela:
```
CMD           TYPE    BEFORE   AFTER    RATIO  LAYER  RTK
cargo test    test    2,341    304      87%    L3     yes
tsc           build   892      241      73%    L2     no
```

`ntk discover` — lê `~/.claude/transcripts/*.jsonl`, analisa comandos Bash sem compressão NTK, imprime oportunidades perdidas:
```
Missed compressions in last session:
  docker logs container_id    ~1,200 tokens (estimated 85% savings)
  cargo clippy                ~800 tokens (estimated 73% savings)
```

**Pronto quando:** ambos os comandos funcionam e imprimem output útil.

---

## FASE 8 — GPU e Performance

### Etapa 21 — Detecção de GPU (`src/gpu.rs`)

**O que fazer:**
```rust
pub enum GpuBackend {
    CudaNvidia { device_id: u32, vram_mb: u64 },
    MetalApple,
    IntelAmx,
    Avx512,
    Avx2,
    CpuScalar,
}

pub fn detect_best_backend() -> GpuBackend
pub fn backend_info(b: &GpuBackend) -> String  // para ntk status
```

Detecção:
- CUDA: verificar `nvidia-smi` ou variável `CUDA_VISIBLE_DEVICES`
- Metal: verificar se target é `aarch64-apple-darwin`
- AMX: verificar `/proc/cpuinfo` flags (Linux) ou CPUID
- AVX-512 / AVX2: CPUID via crate `raw_cpuid`

**Pronto quando:** `ntk status` mostra backend detectado corretamente.

---

### Etapa 22 — Benchmarks (`tests/benchmarks/compression_bench.rs`)

**O que fazer:**
Benchmarks com `criterion`:

```rust
criterion_group!(
    benches,
    bench_layer1_1kb,        // target: < 1ms
    bench_layer1_100kb,      // target: < 5ms
    bench_layer2_tokenizer,  // target: < 20ms
    bench_full_pipeline_no_inference,  // target: < 50ms
);
```

Rodar com: `cargo bench`

**Pronto quando:** todos os benches rodam e relatório HTML é gerado em `target/criterion/`.

---

## FASE 9 — Testes Avançados

### Etapa 23 — Property-based tests (`tests/proptest/compression_invariants.rs`)

**O que fazer:**
```rust
proptest! {
    fn layer1_preserves_error_lines(...)     // erros nunca desaparecem
    fn layer2_reduces_or_equals_tokens(...)  // token count nunca aumenta
    fn compression_is_deterministic(...)     // mesmo input = mesmo output
    fn layer3_not_triggered_below_threshold(...) // threshold respeitado
}
```

**Pronto quando:** todos os 4 testes proptest passam.

---

### Etapa 24 — Snapshot tests (`insta`)

**O que fazer:**
Para cada fixture, criar snapshot do output comprimido:

```rust
#[test]
fn snapshot_cargo_test_layer1() {
    let fixture = include_str!("../fixtures/cargo_test_output.txt");
    insta::assert_snapshot!(layer1_filter(fixture));
}
// idem para tsc, vitest, docker_logs
```

Na primeira execução: `cargo insta review` para aprovar snapshots.
Em execuções futuras: testa regressão.

**Pronto quando:** snapshots aprovados e commits com `tests/snapshots/*.snap`.

---

### Etapa 25 — Quality regression script

**O que fazer:**
`tests/quality_check.sh`:
```bash
#!/bin/bash
PASS=0; FAIL=0
for fixture in tests/fixtures/*.txt; do
    result=$(ntk test-compress "$fixture" --json)
    ratio=$(echo "$result" | jq '.ratio')
    type=$(echo "$result" | jq -r '.output_type')
    # verifica ratio mínimo por tipo
    # verifica que erros do fixture estão no output comprimido
done
echo "Quality check: $PASS passed, $FAIL failed"
[ $FAIL -eq 0 ] || exit 1
```

**Pronto quando:** script roda e verifica todos os fixtures.

---

## FASE 9b — Telemetria

### Etapa 25b — Telemetria anônima (`src/telemetry.rs`)

**O que fazer:**

```rust
pub struct TelemetryReporter {
    config: TelemetryConfig,
    salt_path: PathBuf,     // ~/.ntk/.telemetry_salt
    last_sent_path: PathBuf, // ~/.ntk/.telemetry_last
}

pub struct TelemetryPayload {
    device_hash: String,         // SHA-256(salt + machine_id)
    ntk_version: &'static str,
    os: String,                  // "linux" | "macos" | "windows"
    arch: String,                // "x86_64" | "aarch64"
    commands_24h: u32,
    top_commands: Vec<String>,   // só nomes: ["cargo", "git"] — sem args
    avg_savings_pct: f32,
    layer_distribution: [f32; 3], // [L1%, L2%, L3%]
    gpu_backend: String,         // "cuda" | "metal" | "cpu"
}

impl TelemetryReporter {
    pub async fn maybe_send(&self, metrics: &MetricsStore) -> anyhow::Result<()>
}
```

Regras de implementação:
1. Verificar `std::env::var("NTK_TELEMETRY_DISABLED")` — se `Ok(_)`, retornar imediatamente
2. Verificar `config.telemetry.enabled` — se false, retornar imediatamente
3. Verificar `~/.ntk/.telemetry_last` — se enviado há < 24h, retornar imediatamente
4. Gerar salt em `~/.ntk/.telemetry_salt` se não existe (uuid v4, salvo como hex)
5. Calcular `device_hash = hex(SHA-256(salt || machine_id))`
   - `machine_id`: `/etc/machine-id` (Linux), `IOPlatformUUID` (macOS), registry `MachineGuid` (Windows)
   - Se não disponível: usar string fixa `"unknown"` — hash ainda é anônimo
6. Coletar payload do `MetricsStore` (apenas últimas 24h)
7. POST com timeout de 3s — se falhar, não logar erro visível ao usuário
8. Atualizar `~/.ntk/.telemetry_last` com timestamp atual

Dependências adicionais no `Cargo.toml`:
```toml
sha2 = "0.10"
uuid = { version = "1", features = ["v4"] }
```

**Pronto quando:**
```bash
ntk start
# após 24h ou forçado via ntk telemetry --send-now (debug)
# NTK_TELEMETRY_DISABLED=1 ntk start  → sem envio
```

---

## FASE 10 — CI e Publicação

### Etapa 26 — GitHub Actions CI + Security

**O que fazer:**

`.github/workflows/ci.yml`:

```yaml
name: CI
on: [push, pull_request]
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        rust: [stable]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all
      - run: cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -W clippy::arithmetic_side_effects -D warnings
      - run: cargo fmt --check
      - run: cargo bench --no-run

  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install security tools
        run: cargo install cargo-audit cargo-geiger
      - name: Dependency audit
        run: cargo audit
      - name: Count unsafe blocks
        run: |
          UNSAFE=$(cargo geiger 2>/dev/null | grep -c "unsafe" || true)
          echo "Unsafe blocks found: $UNSAFE"
          # falha se unsafe blocks sem comentário Safety:
          UNCOMMENTED=$(grep -rn "unsafe" src/ | grep -v "// Safety:" | wc -l)
          if [ "$UNCOMMENTED" -gt 0 ]; then
            echo "ERROR: unsafe blocks without // Safety: comment"
            grep -rn "unsafe" src/ | grep -v "// Safety:"
            exit 1
          fi
      - name: Check no unwrap in production code
        run: |
          # unwrap/expect fora de #[cfg(test)] e fora de comentários
          COUNT=$(grep -rn "\.unwrap()\|\.expect(" src/ | grep -v "#\[cfg(test)\]" | grep -v "//.*unwrap" | wc -l)
          if [ "$COUNT" -gt 0 ]; then
            echo "ERROR: unwrap/expect found in production code ($COUNT occurrences)"
            grep -rn "\.unwrap()\|\.expect(" src/ | grep -v "#\[cfg(test)\]" | grep -v "//"
            exit 1
          fi

  release:
    if: startsWith(github.ref, 'refs/tags/')
    strategy:
      matrix:
        include:
          - os: ubuntu-latest,  target: x86_64-unknown-linux-musl
          - os: macos-latest,   target: x86_64-apple-darwin
          - os: macos-latest,   target: aarch64-apple-darwin
          - os: windows-latest, target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: ${{ matrix.target }} }
      - run: cargo build --release --target ${{ matrix.target }}
      - uses: actions/upload-artifact@v4
        with:
          name: ntk-${{ matrix.target }}
          path: target/${{ matrix.target }}/release/ntk*
```

**Pronto quando:** CI passa nos 3 OS + job `security` passa sem erros.

---

## Checklist Final (MVP Completo)

- [ ] **Etapa 1** — Cargo.toml + estrutura de diretórios
- [ ] **Etapa 2** — Config com merge global/local
- [ ] **Etapa 3** — Layer 1 + testes unit
- [ ] **Etapa 4** — Detector + testes unit
- [ ] **Etapa 5** — Layer 2 + testes unit
- [ ] **Etapa 6** — Fixtures reais (6 arquivos)
- [ ] **Etapa 7** — MetricsStore in-memory
- [ ] **Etapa 8** — Servidor HTTP /compress /health /metrics
- [ ] **Etapa 9** — Testes endpoint (axum-test)
- [ ] **Etapa 10** — Testes pipeline (fixtures)
- [ ] **Etapa 11** — CLI com `ntk init` + installer.rs + daemon start/stop
- [ ] **Etapa 12** — Hook scripts (bash + PowerShell) + instaladores (install.sh + install.ps1)
- [ ] **Etapa 13** — Testes CLI (assert_cmd): init/show/uninstall/gain
- [ ] **Etapa 14** — System prompts (4 arquivos)
- [ ] **Etapa 15** — Layer 3 cliente Ollama
- [ ] **Etapa 16** — Mock Ollama (wiremock)
- [ ] **Etapa 17** — Layer 3 integrada no pipeline
- [ ] **Etapa 18** — SQLite persistência (sqlx)
- [ ] **Etapa 19** — Output terminal (graph/table/gain)
- [ ] **Etapa 20** — history + discover
- [ ] **Etapa 21** — Detecção GPU
- [ ] **Etapa 22** — Benchmarks (criterion)
- [ ] **Etapa 23** — Proptest invariantes
- [ ] **Etapa 24** — Snapshot tests (insta)
- [ ] **Etapa 25** — Quality regression script
- [ ] **Etapa 25b** — Telemetria anônima (opt-out)
- [ ] **Etapa 26** — CI cross-platform (ubuntu + macos + windows) + job `security` (audit + unsafe check) + release builds

---

## Ordem de Dependências (Diagrama)

```
1 (Cargo.toml)
└─ 2 (Config)
   ├─ 3 (Layer 1) → 4 (Detector) → 5 (Layer 2)
   │                                └─ 6 (Fixtures)
   └─ 7 (Metrics in-mem)
      └─ 8 (HTTP Daemon) ←── 3, 4, 5
         ├─ 9 (Endpoint tests)
         ├─ 10 (Pipeline tests) ←── 6
         └─ 11 (CLI: ntk init + installer.rs)
            └─ 12 (Hook scripts + instaladores do sistema)
               └─ 13 (CLI tests: init/show/uninstall)
                  └─ 14 (System prompts)
                     └─ 15 (Layer 3 Ollama)
                        └─ 16 (Mock Ollama wiremock)
                           └─ 17 (Layer 3 no pipeline)
                              └─ 18 (SQLite sqlx)
                                 └─ 19 (Output terminal)
                                    ├─ 20 (history/discover)
                                    ├─ 21 (GPU detect)
                                    ├─ 22 (Benchmarks criterion)
                                    ├─ 23 (Proptest)
                                    ├─ 24 (Snapshots insta)
                                    ├─ 25 (Quality script)
                                    ├─ 25b (Telemetria)
                                    └─ 26 (CI + release builds)
```
