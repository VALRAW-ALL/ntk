# NTK — Neural Token Killer
> Evolução semântica do RTK: compressão inteligente de outputs via inferência local

## Status do Projeto (2026-04-11)

**MVP funcional.** Todas as fases de implementação concluídas exceto CI/publicação.

| Componente | Status |
|---|---|
| Layer 1 (Fast Filter) | ✅ Implementado + testes |
| Layer 2 (Tokenizer-Aware) | ✅ Implementado + testes |
| Layer 3 — Ollama | ✅ Implementado + mock tests |
| Layer 3 — Candle (in-process) | ✅ Implementado (CPU/CUDA/Metal) |
| Layer 3 — llama.cpp | ✅ Implementado |
| Daemon HTTP (/compress, /metrics, /health) | ✅ Implementado |
| CLI completa (init, start, stop, status, model, bench) | ✅ Implementado |
| Instalação idempotente (Claude Code + OpenCode) | ✅ Implementado |
| GPU detection | ✅ Implementado |
| SQLite persistence | ✅ Implementado |
| Terminal animado (cores ANSI, spinners) | ✅ Implementado |
| Telemetria anônima | ✅ Implementado |
| Testes (unit, integration, proptest, snapshot) | ✅ Implementado |
| CI / GitHub Actions | ⬜ Pendente |
| Publicação (crates.io / GitHub releases) | ⬜ Pendente |

---

## O que significa NTK?

**NTK = Neural Token Killer**

É uma referência direta ao **RTK (Rust Token Killer)**, projeto que inspirou esta ideia. A troca de "Rust" por "Neural" reflete a evolução da abordagem:

| Sigla | R / N | T | K |
|---|---|---|---|
| **RTK** | **Rust** — linguagem de implementação | **Token** | **Killer** |
| **NTK** | **Neural** — adiciona inferência com rede neural local | **Token** | **Killer** |

Mesma missão (matar tokens desnecessários), nova inteligência. *O RTK, mas com cérebro.*

---

## Visão Geral

O NTK opera como um **proxy de compressão semântica** entre a execução de comandos e o Claude Code. Diferente do RTK (regras estáticas), o NTK usa três camadas progressivas de compressão, ativando inferência local apenas quando necessário.

```
Comando executado
      ↓
  Claude Code (PostToolUse hook)
      ↓
┌─────────────────────────────┐
│         NTK Daemon          │
│                             │
│  Layer 1: Fast Filter       │  → regex/padrões (RTK-style)
│  Layer 2: Tokenizer-Aware   │  → otimiza representação BPE
│  Layer 3: Local Inference   │  → Phi-3 Mini via Ollama ou Candle
│                             │
│  Threshold: só ativa L3     │
│  se output > N tokens       │
└─────────────────────────────┘
      ↓
  Output comprimido → Claude Code
```

**Sem API externa. Sem Claude Haiku. 100% local.**

---

## RTK + NTK: Uso Conjunto

NTK é **complementar** ao RTK, não substituto. Os dois operam em camadas diferentes:

```
Fluxo completo com ambos ativos:

  rtk cargo test
       ↓
  RTK: filtra output no shell (regras estáticas, < 1ms)
       → Remove ANSI, agrupa linhas, mantém só falhas
       ↓
  Claude Code recebe output já filtrado pelo RTK
       ↓
  PostToolUse hook → NTK daemon
       ↓
  NTK Layer 1: detecta output já filtrado, pula redundâncias
  NTK Layer 2: conta tokens — se RTK já reduziu < 300 tokens,
               Layer 3 não é ativada
  NTK Layer 3: se ainda > threshold, inferência semântica
       ↓
  Claude Code recebe output duplamente comprimido
```

**Economia cumulativa esperada:**
- RTK alone: 70–90% de redução
- RTK + NTK: 90–99% de redução (Layer 3 ativa nos casos residuais)

### Comandos de Métricas Combinados

```bash
rtk gain          # Mostra economia RTK da sessão (formato original)
ntk gain          # Mostra economia NTK da sessão (formato RTK-compatível)
ntk discover      # Analisa sessão Claude Code para oportunidades perdidas de RTK/NTK
```

---

## Stack Tecnológico

### Stack Principal

| Componente | Tecnologia | Justificativa |
|---|---|---|
| Daemon/servidor | **Rust + Axum** | Performance determinística, binário único, Tokio-native |
| Tokenizer | `tiktoken-rs` (cl100k_base) | Mesmo tokenizador do Claude/GPT — Layer 2 |
| Tokenizer alternativo | `tokenizers` (HuggingFace) | BPE para modelos não-OpenAI |
| Strip ANSI | `strip-ansi-escapes` | 422k downloads/mês, madura — Layer 1 |
| Modelo local (primário) | **Ollama** + Phi-3 Mini 3.8B Q5_K_M | ~2.8GB RAM, ~200ms/inferência CPU |
| Modelo local (embutido) | **candle** (HuggingFace) + Phi-3 Mini | In-process, sem daemon Ollama, CUDA/Metal |
| Modelo alternativo | `mistral.rs` ou `llama-gguf` | Alta performance, PagedAttention |
| GPU CUDA | `candle-cuda` ou llama.cpp CUDA | RTX: ~50ms/inferência |
| GPU Metal | `candle-metal` ou Ollama Metal | Apple Silicon M1+: ~80ms |
| CPU acelerado | llama.cpp AMX/AVX-512 | Intel Xeon 4th Gen: ~150ms |
| Persistência métricas | `sqlx` + SQLite | Async, queries verificadas em tempo de compilação |
| Output terminal | `ratatui` 0.28+ (modo não-interativo) | Sparklines e tabelas ASCII impressas no stdout |
| Cores e animação | Implementação própria (ANSI + `windows-sys`) | TTY detection, `NO_COLOR`, Spinner braille, BenchSpinner |
| Config | JSON com serde | Simples, editável à mão |
| Integração Claude Code | Hook `PostToolUse` (bash script) | Ponto de entrada oficial |

### Stack de Testes

| Ferramenta | Propósito |
|---|---|
| `criterion` | Benchmarks estatísticos com detecção de regressão |
| `wiremock` | Mock do servidor Ollama em testes de integração |
| `axum-test` | Testes de integração do endpoint `/compress` |
| `proptest` | Testes baseados em propriedades: invariantes de compressão |
| `assert_cmd` | Testes do binário CLI `ntk` |
| `insta` | Snapshot testing para outputs comprimidos |
| `cargo-flamegraph` | Profiling de CPU e identificação de hotspots |

---

## Arquitetura de Camadas

### Layer 1 — Fast Filter (sempre ativo)
- Remove ANSI/cores, progress bars, separadores decorativos (via `strip-ansi-escapes`)
- Detecta se output já foi filtrado pelo RTK (pula processamento redundante)
- Agrupa linhas idênticas: `[×47] same line...`
- Mantém só falhas em outputs de teste (cargo test, vitest, pytest)
- Remove linhas em branco consecutivas
- **Estimativa**: 60-85% de redução para build/test output
- **Latência**: < 1ms

### Layer 2 — Tokenizer-Aware Formatting (sempre ativo)
- Conta tokens reais usando `tiktoken-rs` (modelo cl100k_base)
- Reformata caminhos de arquivo para máxima eficiência BPE:
  ```
  src/components/Button.tsx:10:5 error TS2345
  → Button.tsx:10 TS2345
  ```
- Consolida prefixos repetidos em listas:
  ```
  ERROR: src/a.ts   →   ERROR: a.ts, b.ts, c.ts
  ERROR: src/b.ts
  ERROR: src/c.ts
  ```
- **Estimativa**: +5-15% de redução adicional sobre Layer 1
- **Latência**: < 5ms

### Layer 3 — Local Inference (threshold-based)
- **Ativado apenas se**: output após Layer 1+2 ainda > `inference_threshold_tokens` (default: 300)
- Detecta tipo de output: `test | build | log | diff | generic`
- Envia para backend de inferência com prompt especializado por tipo:
  - **test**: extrair nome dos testes falhos + mensagem de erro
  - **build**: extrair erros de compilação + localização
  - **log**: deduplicar semanticamente + contar ocorrências
  - **diff**: resumir mudanças por arquivo
  - **generic**: resumo estruturado preservando dados acionáveis
- Inclui sempre: primeiro stack trace completo + contagens exatas
- **Estimativa**: 90-99% de redução total
- **Latência**: 30-800ms dependendo do backend (ver tabela de GPU)

### Layer 4 — Context Injection (opcional, avançado)
- O hook passa para o daemon a **intenção atual** do Claude Code (extraída do contexto da tarefa)
- O modelo local filtra com base no que Claude está tentando resolver
- Configurável via `context_aware: true` no config

---

## Aceleração por Hardware (Layer 3)

### Hierarquia de Detecção Automática

O NTK detecta o melhor backend de inferência na inicialização:

```
1. GPU NVIDIA (CUDA)     → candle-cuda ou llama.cpp CUDA
2. GPU Apple (Metal)     → candle-metal ou Ollama Metal
3. Intel AMX (Xeon 4ª Gen / Core Ultra) → llama.cpp AMX
4. AVX-512               → llama.cpp AVX-512
5. AVX2 (padrão x86)     → llama.cpp AVX2
6. Fallback CPU escalar  → configuração mínima
```

### Performance por Hardware (Phi-3 Mini 3.8B, Q5_K_M)

| Backend | Hardware Exemplo | Latência p50 | Latência p95 | RAM/VRAM |
|---|---|---|---|---|
| CUDA | RTX 5060 Ti | ~30ms | ~50ms | 3GB VRAM |
| CUDA | RTX 3060 | ~50ms | ~80ms | 3GB VRAM |
| Metal | M3 MacBook Pro | ~60ms | ~100ms | 3GB RAM unif. |
| Metal | M2 MacBook Air | ~80ms | ~150ms | 3GB RAM unif. |
| Intel AMX | Xeon 4ª Gen | ~150ms | ~250ms | 3GB RAM |
| AVX-512 | i9-13900K | ~200ms | ~350ms | 3GB RAM |
| AVX2 | i7-12700 | ~300ms | ~500ms | 3GB RAM |
| AVX2 | i5-8250U | ~600ms | ~900ms | 3GB RAM |

### Quantizações Recomendadas

| Hardware | Quantização | Tamanho | Justificativa |
|---|---|---|---|
| GPU ≥ 6GB VRAM | Q5_K_M | ~2.8 GB | Melhor qualidade/velocidade |
| GPU < 6GB VRAM | Q4_K_M | ~2.2 GB | Economia de VRAM |
| CPU moderno (≥16GB RAM) | Q5_K_M | ~2.8 GB | Qualidade melhor que Q4 com custo marginal |
| CPU limitado (8GB RAM) | Q4_K_M | ~2.2 GB | Cabe em RAM com folga |

### Opção Candle (In-Process, sem Ollama)

Para ambientes sem Ollama instalado, o NTK pode usar `candle` diretamente:

```toml
# Cargo.toml features
[features]
default = ["ollama"]
cuda = ["candle-core/cuda", "candle-nn/cuda", "candle-transformers/cuda"]
metal = ["candle-core/metal", "candle-nn/metal", "candle-transformers/metal"]
ollama = []  # usa daemon Ollama externo (padrão)
```

```bash
# Compilar com suporte CUDA nativo (sem Ollama)
cargo build --release --features cuda --no-default-features

# Compilar com Metal (Apple Silicon)
cargo build --release --features metal --no-default-features
```

---

## Estrutura do Projeto

```
ntk/
├── src/
│   ├── main.rs              # Entry point do daemon (HTTP server)
│   ├── server.rs            # Rotas HTTP (compress, metrics, health)
│   ├── gpu.rs               # Detecção de GPU/AMX + seleção de backend
│   ├── compressor/
│   │   ├── mod.rs
│   │   ├── layer1_filter.rs # Fast filter + strip-ansi-escapes + RTK detection
│   │   ├── layer2_tokenizer.rs # tiktoken-rs + BPE reformatting
│   │   ├── layer3_backend.rs   # Abstração BackendKind (Ollama/Candle/LlamaCpp)
│   │   ├── layer3_inference.rs # Ollama HTTP client + prompts por tipo
│   │   ├── layer3_candle.rs    # In-process via candle CUDA/Metal/CPU
│   │   └── layer3_llamacpp.rs  # llama.cpp server client com auto-start
│   ├── detector.rs          # Detecta tipo de output (test/build/log/diff)
│   ├── metrics.rs           # sqlx + SQLite persistence + in-memory
│   ├── config.rs            # Deserialização do config.json
│   ├── installer.rs         # ntk init: patch settings.json + copy hook scripts
│   ├── telemetry.rs         # Métricas anônimas opcionais (opt-out NTK_TELEMETRY_DISABLED)
│   └── output/
│       ├── mod.rs
│       ├── terminal.rs      # Cores ANSI, TTY detection, Spinner, BenchSpinner
│       ├── graph.rs         # ASCII bar chart + sparkline no stdout (ratatui não-interativo)
│       └── table.rs         # Tabela de histórico formatada para terminal
├── scripts/
│   ├── ntk-hook.sh          # Hook PostToolUse — Unix (bash)
│   ├── ntk-hook.ps1         # Hook PostToolUse — Windows (PowerShell)
│   ├── install.sh           # One-liner installer — macOS/Linux
│   └── install.ps1          # One-liner installer — Windows
├── tests/
│   ├── unit/
│   │   ├── layer1_tests.rs
│   │   ├── layer2_tests.rs
│   │   └── detector_tests.rs
│   ├── integration/
│   │   ├── compression_pipeline_tests.rs
│   │   ├── ollama_mock_tests.rs      # wiremock-rs mock do servidor Ollama
│   │   ├── endpoint_tests.rs         # axum-test do endpoint /compress
│   │   └── cli_tests.rs              # assert_cmd para CLI ntk
│   ├── proptest/
│   │   └── compression_invariants.rs # proptest: invariantes de compressão
│   ├── snapshots/
│   │   └── *.snap                    # insta snapshot outputs
│   ├── benchmarks/
│   │   └── compression_bench.rs      # criterion.rs
│   └── fixtures/
│       ├── cargo_test_output.txt
│       ├── cargo_test_rtk_filtered.txt  # output já filtrado pelo RTK
│       ├── tsc_output.txt
│       ├── vitest_output.txt
│       ├── next_build_output.txt
│       └── docker_logs.txt
├── config/
│   └── default_config.json  # Config padrão documentado
├── system-prompts/
│   ├── test.txt
│   ├── build.txt
│   ├── log.txt
│   └── diff.txt
└── Cargo.toml
```

---

## Configuração Global (`~/.ntk/config.json`)

```json
{
  "version": "1.0",
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
    "context_aware": false,
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
    "temperature": 0.1,
    "gpu_layers": -1,
    "gpu_auto_detect": true,
    "cuda_device": 0,
    "llama_cpp_path": null
  },
  "metrics": {
    "enabled": true,
    "storage_path": "~/.ntk/metrics.db",
    "history_days": 30,
    "track_per_command": true,
    "track_per_session": true
  },
  "output_types": {
    "test": {
      "layer3_prompt": "system-prompts/test.txt",
      "threshold_override": 200
    },
    "build": {
      "layer3_prompt": "system-prompts/build.txt",
      "threshold_override": 250
    },
    "log": {
      "layer3_prompt": "system-prompts/log.txt",
      "threshold_override": 400
    },
    "diff": {
      "layer3_prompt": "system-prompts/diff.txt",
      "threshold_override": 350
    }
  },
  "exclusions": {
    "commands": ["cat", "echo", "pwd"],
    "max_input_chars": 100000
  },
  "display": {
    "show_compression_ratio": true,
    "show_layer_used": false,
    "show_backend": true,
    "color": true
  }
}
```

### Configuração por Projeto (`.ntk.json` na raiz do projeto)

```json
{
  "compression": {
    "inference_threshold_tokens": 150
  },
  "model": {
    "model_name": "phi3:medium",
    "quantization": "q5_k_m"
  },
  "exclusions": {
    "commands": ["prisma studio"]
  }
}
```

---

## Instalação do Sistema

### One-liner (recomendado)

```bash
# macOS / Linux
curl -sSf https://raw.githubusercontent.com/user/ntk/main/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/user/ntk/main/install.ps1 | iex

# Qualquer plataforma — Cargo
cargo install ntk
```

Após instalar o binário, configurar o hook:

```bash
ntk init -g          # instala hook + config (recomendado)
```

### Plataformas de distribuição

| Método | Plataforma | Status |
|---|---|---|
| `install.sh` (curl) | macOS, Linux | MVP |
| `install.ps1` (PowerShell) | Windows | MVP |
| `cargo install ntk` | Todas | MVP |
| Homebrew formula | macOS, Linux | Pós-MVP |
| Winget package | Windows | Pós-MVP |

---

## CLI (`ntk <comando>`)

### Init (instalação e configuração)

| Comando | Descrição |
|---|---|
| `ntk init -g` | Instala globalmente: hook + `~/.ntk/config.json` (recomendado) |
| `ntk init -g --opencode` | Hook para OpenCode em vez de Claude Code |
| `ntk init -g --auto-patch` | Não-interativo — sem prompts (CI/CD) |
| `ntk init -g --hook-only` | Só instala o hook, sem criar config |
| `ntk init --show` | Mostra status completo da instalação |
| `ntk init --uninstall` | Remove hook do settings.json do editor |

`ntk init -g` é **idempotente** — rodar duas vezes não duplica o hook.

### Daemon e compressão

| Comando | Descrição |
|---|---|
| `ntk start` | Inicia o daemon em background |
| `ntk start --gpu` | Inicia com inferência GPU ativa |
| `ntk stop` | Para o daemon |
| `ntk status` | Status do daemon + modelo + backend GPU |
| `ntk test-compress <file>` | Testa compressão em um arquivo de output |
| `ntk gain` | Resumo de economia de tokens (compatível com RTK) |
| `ntk metrics` | Tabela de métricas da sessão atual |
| `ntk graph` | ASCII bar chart + sparkline no stdout (não-interativo) |
| `ntk history` | Histórico dos últimos N comandos comprimidos |
| `ntk discover` | Analisa sessão para oportunidades perdidas de RTK/NTK |

### Modelo

| Comando | Descrição |
|---|---|
| `ntk model pull` | Baixa phi3:mini via Ollama (~2GB) |
| `ntk model pull --quant q5_k_m` | Baixa quantização específica |
| `ntk model test` | Testa latência e qualidade do modelo |
| `ntk model bench` | Benchmark CPU vs GPU por backend disponível |

### Configuração

| Comando | Descrição |
|---|---|
| `ntk config` | Abre `~/.ntk/config.json` no `$EDITOR` |

---

## Integração com Claude Code

### Hook `PostToolUse` (`~/.claude/settings.json`)

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "~/.ntk/bin/ntk-hook.sh"
          }
        ]
      }
    ]
  }
}
```

### Schema JSON do Hook (stdin → hook)

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

### Script do Hook (`ntk-hook.sh`)

```bash
#!/bin/bash
# Lê o output do tool result via stdin (JSON do Claude Code)
# Extrai o campo de output
# Envia para o daemon NTK na porta 8765
# Retorna o output comprimido

INPUT=$(cat)
OUTPUT=$(echo "$INPUT" | jq -r '.tool_response.output // .output // empty')

if [ -z "$OUTPUT" ] || [ ${#OUTPUT} -lt 500 ]; then
  echo "$INPUT"
  exit 0
fi

COMPRESSED=$(curl -sf -X POST http://127.0.0.1:8765/compress \
  -H "Content-Type: application/json" \
  -d "{\"output\": $(echo "$OUTPUT" | jq -Rs .), \"context\": \"$NTK_CONTEXT\"}" \
  | jq -r '.compressed // empty')

if [ -n "$COMPRESSED" ]; then
  echo "$INPUT" | jq --arg c "$COMPRESSED" '.tool_response.output = $c'
else
  echo "$INPUT"
fi
```

---

## Prompts do Modelo Local (por tipo de output)

### `system-prompts/test.txt`
```
You are a test output compressor. Given raw test runner output, extract ONLY:
1. Number of tests: X passed, Y failed, Z skipped
2. For each FAILED test: test name + exact error message + file:line
3. Total duration

Discard: passing test names, progress bars, coverage tables, warnings.
Output in compact format. No prose.
```

### `system-prompts/build.txt`
```
You are a build output compressor. Extract ONLY:
1. Build result: success/failed
2. For each ERROR: file:line + error code + message (1 line)
3. Warning count only (not individual warnings unless < 3)
4. Build duration if present

Discard: info messages, progress, module counts, asset sizes (unless asked).
```

### `system-prompts/log.txt`
```
You are a log compressor. Extract:
1. ERROR and CRITICAL lines (all, with timestamp and count if repeated)
2. WARN lines grouped: "[×N] message" if same message repeats
3. Any exception/stack trace (first occurrence only)
4. Summary: X errors, Y warnings in N lines

Discard: INFO, DEBUG, TRACE lines unless they appear in the 3 lines before an error.
```

---

## Plano de Testes

### Unit Tests

**`layer1_tests.rs`**
```rust
#[test]
fn test_remove_ansi_codes() { ... }

#[test]
fn test_group_repeated_lines() { ... }

#[test]
fn test_keep_only_failures_cargo_test() { ... }

#[test]
fn test_remove_progress_bars() { ... }

#[test]
fn test_collapse_blank_lines() { ... }

#[test]
fn test_detect_rtk_filtered_output() { ... }  // detecta output já filtrado pelo RTK

#[test]
fn test_skip_redundant_processing_on_rtk_output() { ... }
```

**`layer2_tests.rs`**
```rust
#[test]
fn test_token_count_accuracy() { ... }  // compara com tiktoken-py

#[test]
fn test_path_shortening_reduces_tokens() { ... }

#[test]
fn test_prefix_consolidation() { ... }

#[test]
fn test_no_data_loss_in_reformatting() { ... }

#[test]
fn test_threshold_not_triggered_below_300() { ... }

#[test]
fn test_threshold_triggered_above_300() { ... }
```

**`detector_tests.rs`**
```rust
#[test]
fn test_detects_cargo_test_output() { ... }

#[test]
fn test_detects_tsc_output() { ... }

#[test]
fn test_detects_vitest_output() { ... }

#[test]
fn test_detects_docker_logs() { ... }

#[test]
fn test_detects_git_diff() { ... }

#[test]
fn test_unknown_falls_back_to_generic() { ... }
```

### Integration Tests

**`compression_pipeline_tests.rs`**
```rust
// Testa pipeline completo Layer 1 → 2 → 3 com fixtures reais
#[test]
fn test_cargo_test_fixture_compression_ratio() {
    // Carrega fixture de 800 linhas
    // Roda pipeline
    // Assert: ratio >= 0.85 (85% de redução)
    // Assert: erros presentes no output final
}

#[test]
fn test_cargo_test_rtk_pre_filtered() {
    // Fixture já filtrada pelo RTK
    // Layer 3 não deve ativar (< 300 tokens pós RTK)
}

#[test]
fn test_next_build_fixture_errors_preserved() { ... }

#[test]
fn test_layer3_not_triggered_below_threshold() { ... }

#[test]
fn test_layer3_triggered_above_threshold() { ... }
```

**`ollama_mock_tests.rs`** (via wiremock-rs)
```rust
// Mock do servidor Ollama para testes sem GPU/Ollama instalado
#[tokio::test]
async fn test_inference_request_format() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
        .mount(&mock_server)
        .await;
    // ...
}

#[tokio::test]
async fn test_fallback_on_ollama_timeout() { ... }

#[tokio::test]
async fn test_fallback_on_ollama_unavailable() { ... }
```

**`endpoint_tests.rs`** (via axum-test)
```rust
// Testa o endpoint /compress do daemon diretamente
#[tokio::test]
async fn test_compress_endpoint_returns_compressed() {
    let server = TestServer::new(app()).unwrap();
    let response = server.post("/compress")
        .json(&json!({ "output": cargo_test_fixture }))
        .await;
    response.assert_status_ok();
    assert!(response.json::<Value>()["compressed"].as_str().unwrap().len() < cargo_test_fixture.len());
}
```

**`cli_tests.rs`** (via assert_cmd)
```rust
// Testa o binário ntk como um processo externo
#[test]
fn test_ntk_status_shows_daemon_info() {
    Command::cargo_bin("ntk")
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("daemon"));
}
```

### Property-Based Tests (proptest)

**`compression_invariants.rs`**
```rust
use proptest::prelude::*;

proptest! {
    // Erros jamais devem desaparecer após compressão
    #[test]
    fn layer1_preserves_error_lines(input in any_output_with_errors()) {
        let compressed = layer1_filter(&input);
        for error in extract_errors(&input) {
            prop_assert!(compressed.contains(&error));
        }
    }

    // Token count sempre cai após Layer 2
    #[test]
    fn layer2_reduces_or_equals_tokens(input in any_text_output()) {
        let (compressed, original_tokens, new_tokens) = layer2_process(&input);
        prop_assert!(new_tokens <= original_tokens);
    }

    // Compressão é determinística
    #[test]
    fn compression_is_deterministic(input in any_text_output()) {
        let a = layer1_filter(&input);
        let b = layer1_filter(&input);
        prop_assert_eq!(a, b);
    }
}
```

### Snapshot Tests (insta)

```rust
// tests/snapshots/*.snap — aprovados manualmente via `cargo insta review`
#[test]
fn test_cargo_test_snapshot() {
    let fixture = include_str!("../fixtures/cargo_test_output.txt");
    let compressed = run_layer1(fixture);
    insta::assert_snapshot!(compressed);
}
```

### Benchmarks (`compression_bench.rs` com criterion)

```rust
criterion_group!(
    benches,
    bench_layer1_1kb,
    bench_layer1_100kb,
    bench_layer2_tokenizer,
    bench_full_pipeline_no_inference,
    bench_full_pipeline_with_inference,  // requer Ollama rodando
    bench_gpu_inference,                 // requer GPU disponível
);
```

**Targets de performance:**
- Layer 1 em 100KB: < 5ms
- Layer 2 em 100KB: < 20ms
- Layer 3 (CPU AVX2): < 1000ms p95
- Layer 3 (GPU CUDA RTX 3060+): < 100ms p95
- Layer 3 (Metal Apple Silicon): < 150ms p95

### Testes de Regressão de Qualidade

Script `tests/quality_check.sh`:
```bash
# Para cada fixture em tests/fixtures/:
# 1. Comprime com NTK
# 2. Verifica se erros/falhas originais estão presentes no output
# 3. Verifica ratio mínimo de compressão por tipo
# 4. Compara com fixture RTK-pré-filtrada (valida detecção de RTK output)
# 5. Gera relatório comparativo
```

---

## Gráficos de Métricas no Terminal (`ntk graph`)

Output via `ratatui` no modo não-interativo (imprime no stdout e encerra). Sem TUI interativa — todos os comandos retornam imediatamente.

```
$ ntk graph

NTK Metrics — Sessão atual           Total economizado: 12,847 tokens
Backend: CUDA (RTX 3060)             Layer 3 latência média: 52ms

Tokens por Comando (últimos 10)
cargo test   ████████████████░░░░  87%  (2,341 → 304)  [L3]
tsc          ████████████░░░░░░░░  73%  (892 → 241)    [L2]
git log      ████████░░░░░░░░░░░░  61%  (445 → 174)    [L1]
next build   ██████████████████░░  91%  (5,120 → 461)  [L3]
vitest       ████████████████████  99%  (3,892 → 39)   [L3]
docker logs  ███████████░░░░░░░░░  68%  (712 → 228)    [L3]

Layer utilizada
L1 (fast)   ████████████░  48%
L2 (token)  █████░         21%
L3 (model)  ███████░       31%

Economia acumulada (7 dias)
45k ┤                                              ╭───
30k ┤                               ╭──────────────╯
15k ┤              ╭────────────────╯
 0k ┤──────────────╯
     seg   ter   qua   qui   sex   sab   dom
```

---

## Plano de Testes — Detecção de Tokens Inferidos vs Claude Code

Para medir o impacto real do NTK no contexto do Claude Code:

### Métricas a Coletar

```
Por sessão Claude Code:
  - Tokens antes da compressão (estimado via tiktoken-rs)
  - Tokens após a compressão (medido)
  - Layer utilizada por comando
  - Latência adicional introduzida pelo NTK
  - Contexto economizado (tokens / janela de contexto do Claude)

Por comando:
  - Tipo detectado (test/build/log/diff/generic)
  - Ratio de compressão
  - Tempo de resposta Layer 3
  - Backend utilizado (CPU/CUDA/Metal/AMX)
  - Se RTK pré-filtrou o output (flag: rtk_pre_filtered)
```

### Endpoint de Análise (`/metrics/session`)

```json
{
  "session_id": "abc123",
  "total_tokens_saved": 12847,
  "total_commands_compressed": 23,
  "rtk_pre_filtered": 8,
  "by_layer": { "L1": 11, "L2": 5, "L3": 7 },
  "by_type": { "test": 8, "build": 6, "log": 5, "diff": 4 },
  "avg_latency_ms": { "L1": 0.3, "L2": 3.1, "L3": 287 },
  "backend": "cuda",
  "context_window_saved_pct": 34.2
}
```

### `ntk discover` — Análise Retrospectiva

```bash
ntk discover
# Analisa ~/.claude/transcripts/*.jsonl
# Identifica comandos que NTK poderia ter comprimido e não comprimiu
# Identifica comandos RTK usados + potencial adicional do NTK
# Saída: relatório com oportunidades perdidas e economia potencial
```

---

## Skills, Rules e Agentes por Fase

Skills e regras do Claude Code mapeadas às fases de implementação do NTK.

### Skills Disponíveis

| Skill | Trigger | Fase NTK |
|---|---|---|
| **`write-tests`** | Ao implementar qualquer módulo novo | Todas as fases com código |
| **`clean-code`** | Ao finalizar cada etapa | Todas |
| **`architecture-review`** | Antes de fases críticas de design | 3, 5, 8 |
| **`dead-code`** | Após integração completa de uma fase | Pós-17, pós-26 |
| **`brainstorm`** | Ao encontrar decisão não coberta no roadmap | Qualquer fase |

### Rules Aplicadas Automaticamente

**`clean-code` (sempre ativo em Rust):**
- Funções com responsabilidade única
- Nomes descritivos sem abreviações (`layer1_filter`, não `l1f`)
- `pub` apenas no necessário — minimizar surface pública
- Erros via `thiserror` para tipos próprios, `anyhow` para propagação
- Sem `unwrap()` em código de produção

**`write-tests` (ao criar módulo):**
- Unit tests no mesmo arquivo (`#[cfg(test)]`) para funções puras
- Integration tests em `tests/integration/` para endpoints e pipeline
- Proptest para invariantes de compressão
- Snapshots insta para regressão de output

**`architecture-review` (antes de fases 3, 5, 8):**
- Valida que não há dependências circulares entre módulos
- Verifica que Layer 3 é opcional (compilação sem Ollama funciona)
- Confirma que todas as APIs são `async` onde necessário

### Mapa de Responsabilidade por Fase

```
Fase 1-2  — Config e estrutura
  Skills: clean-code
  Validação: cargo check passa nos 3 OS

Fase 2    — Compressão core (Layer 1, 2, Detector)
  Skills: write-tests (após cada layer), clean-code
  Validação: todos unit tests passam + clippy limpo

Fase 3    — Daemon HTTP
  Skills: architecture-review (schema endpoint), write-tests
  Agente: Plan — validar estrutura do pipeline antes de implementar
  Validação: endpoint tests passam + curl manual funciona

Fase 4    — CLI
  Skills: write-tests (assert_cmd), clean-code
  Validação: ntk install/start/stop/status funciona nos 3 OS

Fase 5    — Layer 3 / Ollama
  Skills: write-tests (wiremock mock), brainstorm se qualidade < 70%
  Agente: Plan — validar fallback logic e threshold
  Validação: mock tests passam sem Ollama instalado

Fase 6    — SQLite
  Skills: clean-code
  Validação: dados persistem após restart do daemon

Fase 7    — Output terminal
  Skills: clean-code
  Validação: ntk graph/metrics/gain imprimem e retornam ao prompt

Fase 8    — GPU
  Skills: architecture-review (abstração GpuBackend)
  Validação: ntk status mostra backend correto em cada OS

Fase 9    — Testes avançados
  Skills: write-tests (proptest + insta), dead-code
  Validação: 0 falhas em proptest + snapshots aprovados

Fase 10   — CI
  Skills: dead-code (limpeza final)
  Validação: CI verde nos 3 OS (ubuntu, macos, windows)
```

---

## MVP — Escopo Mínimo Testável

**Objetivo**: Demonstrar redução real de tokens em casos de uso comuns do Claude Code.

### Sprint 1 — Core (sem modelo)
- [ ] Daemon HTTP básico em Rust com Axum (start/stop/health)
- [ ] Layer 1: fast filter com `strip-ansi-escapes` + detecção RTK output
- [ ] Layer 2: contagem de tokens com `tiktoken-rs`
- [ ] Detector de tipo de output (test/build/log/generic)
- [ ] Endpoint `/compress` funcional
- [ ] `ntk init -g` funcional (hook + config, idempotente, 3 OS)
- [ ] Hook `ntk-hook.sh` + `ntk-hook.ps1` funcionando com Claude Code
- [ ] Métricas em memória (tokens antes/depois)
- [ ] `ntk metrics` no terminal (tabela simples)
- [ ] Config `~/.ntk/config.json` básico
- [ ] Testes unit para Layer 1 + detector + proptest invariantes
- [ ] Snapshot tests com insta para Layer 1 output

### Sprint 2 — Inferência Local
- [ ] Cliente Ollama (HTTP) com timeout e fallback
- [ ] Prompts para: test, build, log, diff
- [ ] Layer 3 com threshold configurável
- [ ] Fallback automático se Ollama indisponível
- [ ] Detecção automática de GPU (CUDA/Metal/AMX)
- [ ] Config `gpu_layers` + `gpu_auto_detect`
- [ ] `ntk model pull` + `ntk model test` + `ntk model bench`
- [ ] Testes integração com wiremock-rs (mock Ollama)
- [ ] Testes endpoint com axum-test
- [ ] Benchmarks com criterion (L1/L2/L3)

### Sprint 3 — Métricas, GPU e UX
- [ ] `sqlx` + SQLite para persistência de métricas
- [ ] Suporte `candle` in-process (CUDA/Metal sem Ollama)
- [ ] `ntk graph` com ratatui 0.28+ (sparkline + bar charts + info GPU)
- [ ] `ntk gain` compatível com formato RTK
- [ ] `ntk history` (últimos 20 comandos com layer e backend)
- [ ] `ntk discover` (análise retrospectiva de sessões)
- [ ] `ntk init --show` / `ntk init --uninstall`
- [ ] Telemetria anônima com opt-out (`NTK_TELEMETRY_DISABLED=1`)
- [ ] Instaladores do sistema (`install.sh` + `install.ps1`)
- [ ] Testes CLI com assert_cmd
- [ ] Testes de regressão de qualidade com fixtures reais (incluindo RTK-pré-filtradas)

### Critérios de Segurança do MVP (não-negociáveis)
- `cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used -D warnings` passa sem erros
- `cargo audit` sem vulnerabilidades conhecidas de severidade alta/crítica
- Zero `unwrap()`/`expect()` fora de `#[cfg(test)]`
- `ollama_url` validado como localhost ao carregar config
- Escrita em `settings.json` atômica (via rename)
- Permissões `600` em config e salt da telemetria
- `NTK_TELEMETRY_DISABLED=1` desativa completamente a telemetria

### Critérios de Sucesso do MVP
- Redução média > 70% em outputs de `cargo test` e `tsc`
- Redução média > 90% com Layer 3 ativa para logs verbosos
- Latência total < 50ms para Layer 1+2
- Latência total < 1s para Layer 3 em CPU (p95)
- Latência total < 100ms para Layer 3 em GPU CUDA (p95)
- Zero perda de informação de erro em 100% dos fixtures de teste
- Funciona sem modelo instalado (fallback gracioso para Layer 1+2)
- Detecta e evita reprocessamento de output já filtrado pelo RTK

---

## Instalação (fluxo do usuário final)

```bash
# 1. Instalar binário (escolha um)
curl -sSf https://...install.sh | sh          # macOS/Linux (recomendado)
irm https://...install.ps1 | iex              # Windows PowerShell
cargo install ntk                              # qualquer plataforma

# 2. Configurar hook no Claude Code (ou OpenCode)
ntk init -g                    # Claude Code (padrão)
ntk init -g --opencode         # OpenCode
ntk init -g --auto-patch       # sem prompts (CI/CD)

# 3. Verificar instalação
ntk init --show
# → Binary:      /usr/local/bin/ntk (v0.1.0)       ✓
# → Hook script: ~/.ntk/bin/ntk-hook.sh             ✓
# → Config:      ~/.ntk/config.json                 ✓
# → Editor hook: ~/.claude/settings.json            ✓
# → Daemon:      stopped
# → Model:       not installed

# 4. Instalar modelo (opcional mas recomendado para Layer 3)
ntk model pull
# ou para melhor qualidade:
ntk model pull --quant q5_k_m

# 5. Iniciar daemon
ntk start

# 6. Verificar status com GPU
ntk status
# → NTK daemon running on :8765
# → Model: phi3:mini (Q5_K_M) via Ollama
# → Backend: CUDA (NVIDIA RTX 3060, 12GB VRAM)
# → GPU layers: 32/32 (full GPU mode)

# 7. Usar normalmente — compressão é transparente
ntk graph    # ver métricas da sessão
ntk gain     # ver economia de tokens
```

---

## Segurança

### Superfícies de Ataque e Mitigações

| Superfície | Vetor de Ataque | Mitigação |
|---|---|---|
| Hook stdin | JSON malformado ou gigante | Limite `max_input_chars` + leitura com tamanho máximo |
| `/compress` endpoint | Body gigante → OOM | `max_input_chars` enforced no handler Axum |
| `ollama_url` no config | SSRF para rede interna | Validar localhost-only ao carregar `config.rs` |
| `ntk test-compress <file>` | Path traversal (`../../etc/passwd`) | `canonicalize()` + verificar que não é device file |
| System prompts | Injeção via arquivo modificado | Carregar só de `~/.ntk/system-prompts/` hardcoded |
| `settings.json` patch | Corrupção em crash no meio do write | Escrita atômica via `write tmp → rename` |
| PID file | TOCTOU race condition | Verificar PID ativo antes de sinalizar |
| Layer 3 | Prompt injection via output comprimido | Conteúdo do usuário só no user turn, nunca no system prompt |
| Telemetria | Vazamento de paths/args | `NTK_TELEMETRY_DISABLED` verificado antes de coletar |
| Config JSON | Overflow em valores numéricos | Bounds validation em `config.rs` |

### Lints de Segurança Obrigatórios

```bash
# Ativados em todos os builds de CI
cargo clippy -- \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic \
  -W clippy::arithmetic_side_effects \
  -D warnings

# Auditoria de dependências (a cada mudança no Cargo.toml)
cargo audit

# Contagem de unsafe blocks (deve ser 0 ou cada um comentado com // Safety:)
cargo geiger
```

### Propriedades de Segurança por Design (Rust)

- **Memory safety**: sem buffer overflow, use-after-free, ou null pointer por design da linguagem
- **Thread safety**: `Send`/`Sync` verificados em tempo de compilação — sem data races
- **Sem GC**: sem pauses imprevisíveis que possam causar timeout na compressão
- **`unsafe` zero por padrão**: qualquer bloco unsafe deve ter comentário `// Safety:` explicando a invariante

---

## Privacidade e Telemetria

O NTK coleta métricas de uso **anônimas e agregadas** uma vez por dia. Recurso ativado por padrão para ajudar a priorizar o desenvolvimento.

### O que é coletado

- Hash do dispositivo (SHA-256 com salt — salt aleatório por usuário armazenado localmente, não reversível)
- Versão NTK, SO, arquitetura
- Contagem de comandos (últimas 24h) e nomes dos comandos mais usados (ex: `"cargo"`, `"git"` — sem argumentos, sem caminhos)
- Percentagem de economia de tokens
- Distribuição de layers (L1/L2/L3 %)
- Backend GPU utilizado (ex: `"cuda"`, `"metal"`, `"cpu"`)

### O que NÃO é coletado

Código-fonte, caminhos de arquivos, argumentos de comandos, segredos, variáveis de ambiente ou qualquer informação pessoal identificável.

### Desativar telemetria (qualquer uma das opções)

```bash
# Variável de ambiente (sessão ou ~/.bashrc / ~/.zshrc)
export NTK_TELEMETRY_DISABLED=1

# Ou no arquivo de config (~/.ntk/config.json)
{
  "telemetry": { "enabled": false }
}
```

### Implementação (`src/telemetry.rs`)

- Um HTTP POST por dia para endpoint de telemetria
- Verifica `NTK_TELEMETRY_DISABLED` primeiro — se definido, não envia nada
- Salt em `~/.ntk/.telemetry_salt` (gerado uma vez, nunca enviado)
- Hash do dispositivo = `SHA-256(salt + machine_id)` — não reversível
- Fire-and-forget: falha na telemetria nunca bloqueia o pipeline de compressão
- Timeout: 3 segundos

---

## Compatibilidade Cross-Platform

NTK deve compilar e rodar em **Windows, macOS e Linux** sem modificações de código.

### Regras Obrigatórias

| Regra | Motivo |
|---|---|
| TCP `127.0.0.1:8765` (não Unix socket) | Unix sockets não existem no Windows |
| `dirs` crate para caminhos de home/config | Nunca hardcodar `/home/` ou `C:\Users\` |
| `std::path::PathBuf` para todos os paths | Evita separadores `/` vs `\` hardcodados |
| `std::process::Command` para subprocessos | Nunca usar `fork()`/`libc` diretamente |
| `sqlx` com feature `bundled` (SQLite) | SQLite embutido no binário, sem dep externa |
| GPU opcional via feature flags | `cargo build` sem flags → binário funcional CPU-only |

### Hook por OS

```
macOS/Linux:  ~/.ntk/bin/ntk-hook.sh       (bash)
Windows:      ~/.ntk/bin/ntk-hook.ps1      (PowerShell)
```

`ntk install` detecta o OS e configura o hook correto em `~/.claude/settings.json`.

### CI Matrix

```yaml
# .github/workflows/ci.yml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
    rust: [stable]
```

Todos os testes (unit + integration) devem passar nos 3 OS.

---

## Notas de Design

**Por que Rust?**
Latência determinística, binário único sem dependências de runtime, compilação nativa para Windows/macOS/Linux, integração natural com tiktoken-rs e strip-ansi-escapes.

**Por que sem TUI interativa?**
TUI interativa (modo alternado de tela, captura de teclado) cria problemas em ambientes CI, pipes, e terminais Windows sem suporte completo a ANSI. Todos os comandos NTK imprimem no stdout e encerram — composáveis com `|`, `>`, e scripts.

**Por que Phi-3 Mini Q5_K_M?**
Melhor custo-benefício qualidade/velocidade para tasks de sumarização estruturada. Q5_K_M oferece qualidade significativamente melhor que Q4 com apenas 20% mais tamanho. Roda em CPU (300-600ms) ou GPU (30-80ms). Alternativas: Gemma 3 2B (menor, ~1.5GB), Llama 3.2 3B (mais capaz), Qwen 3 Small (multilíngue, bom para PT-BR).

**Por que não comprimir sempre com modelo?**
Latência. Para um `git status` de 10 linhas, o overhead de 300ms é inaceitável. O threshold garante que Layer 3 só ativa onde o ROI de tokens justifica o custo de tempo.

**Por que Candle além de Ollama?**
Candle permite inferência in-process sem depender de um daemon externo. Útil em ambientes sem Ollama, CI/CD, ou quando se deseja controle total da latência. A troca: Ollama é mais fácil de instalar e gerencia modelos automaticamente; Candle requer build com feature flags mas elimina a dependência de processo externo.

**Por que sqlx em vez de rusqlite?**
O daemon é async (Tokio/Axum). Writes de métricas não podem bloquear a thread do runtime. sqlx provê SQLite async com queries verificadas em tempo de compilação.

**Por que proptest para testes?**
Compressão tem invariantes que testes manuais dificilmente cobrem (entradas arbitrárias, Unicode, outputs gigantes). proptest gera centenas de casos e minimiza automaticamente inputs que causam falha.

**Compatibilidade com RTK**
NTK detecta output já filtrado pelo RTK (sem ANSI, linhas agrupadas) e pula processamento redundante na Layer 1. O comando `ntk gain` usa formato idêntico ao `rtk gain` para facilitar migração. NTK e RTK coexistem: RTK filtra no shell, NTK comprime o resultado via hook.
