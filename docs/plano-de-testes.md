# NTK — Plano de Testes de Redução de Tokens

> Documento de planejamento para medir o impacto real do NTK no consumo
> de tokens do Claude Code. Não é spec de implementação — cada seção
> termina com perguntas em aberto que precisam ser fechadas antes de
> começar a codar.

---

## Objetivo

Quantificar, com números reproduzíveis, quanto o NTK realmente reduz
os tokens cobrados pela API da Anthropic quando o Claude Code roda
ferramentas Bash. Produzir evidência para:

1. **Contribuição por camada** — quantos tokens o L1 remove? L2? L3?
2. **Economia ponta a ponta** — rodar o mesmo prompt com e sem o hook
   NTK; medir o delta em `usage.input_tokens` registrado pelo próprio
   Claude Code.
3. **Onde o NTK vale a pena** — quais outputs de ferramenta têm a
   maior redução? Existem categorias em que o NTK adiciona latência
   sem economizar tokens o suficiente?

Fora de escopo (neste plano):

- Medir qualidade semântica / fidelidade do output do L3.
- Benchmarks de latência do llama.cpp vs Ollama.
- Avaliar UX do wizard / instalador.

---

## Dados que já temos

| Sinal | Fonte | Observações |
|---|---|---|
| Contagens de tokens por compressão | Resposta do `POST /compress`: `tokens_before`, `tokens_after`, `layer`, `ratio` | Só os números da camada **final**; os intermediários de L1/L2 são perdidos. |
| Métricas agregadas da sessão | `GET /metrics` — `{total_original_tokens, total_compressed_tokens, layer_counts, average_ratio}` | Por sessão, em memória, resetado quando o daemon reinicia. |
| Histórico de compressões | `~/.ntk/metrics.db` (SQLite) | Linhas persistidas por compressão; schema em `src/metrics.rs`. |
| Uso de tokens por turno no Claude Code | `~/.claude/projects/<projeto>/<session-id>.jsonl` | Cada evento `assistant` tem `message.usage.{input_tokens, cache_read_input_tokens, cache_creation_input_tokens, output_tokens}`. |

## Dados que AINDA não temos

| Sinal faltante | Por que precisa | Como obter |
|---|---|---|
| Input bruto recebido pelo daemon | Impossível replay / auditoria de compressões | Adicionar persistência no handler `POST /compress`. |
| Output após o L1 (antes do L2 rodar) | Impossível atribuir economia só ao L1 | Expor na resposta + salvar snapshot. |
| Output após o L2 (antes do L3 rodar) | Mesma coisa para o L2 | Idem. |
| Output comprimido final (persistido) | Impossível reproduzir o que foi enviado ao Claude | Salvar junto com o input bruto. |

---

## Arquitetura do harness de testes

```
┌─ microbench (replay de fixtures) ─┐  ┌─ macrobench (sessão real) ──────┐
│ bench/fixtures/*.txt              │  │ bench/prompts/baseline.md       │
│   ↓                               │  │   ↓                             │
│ bench/replay.sh                   │  │ Run A: hook desinstalado        │
│   para cada fixture:              │  │   → transcripts/A.jsonl         │
│     POST /compress                │  │ Run B: hook ativo               │
│     grava input/output/stats      │  │   → transcripts/B.jsonl         │
│       em ~/.ntk/logs/...          │  │   → ~/.ntk/logs/B/*.json        │
│   produz microbench.csv           │  │ bench/parse_transcript.py       │
└───────────────────────────────────┘  │   produz macrobench.csv         │
                                       └─────────────────────────────────┘
                                                     ↓
                               bench/report.py → relatorio.md
                                 - histograma de economia por camada
                                 - tabela de ratio por fixture
                                 - delta A-vs-B (tokens + $ estimado)
                                 - tabela de overhead (latência por layer)
```

---

## Passos de implementação (ordenados, cada um entregável independente)

### Passo 1 — Expor métricas por camada no `/compress`

**Arquivos:** `src/server.rs` (`CompressResponse`), `src/compressor/mod.rs`

Estender a resposta:

```json
{
  "compressed":       "<output final>",
  "layer":            3,
  "tokens_before":    3200,
  "tokens_after_l1":  1800,
  "tokens_after_l2":  1200,
  "tokens_after_l3":   280,
  "tokens_after":      280,
  "ratio":            0.91,
  "latency_ms": { "l1": 4, "l2": 12, "l3": 820 }
}
```

`tokens_after_l2` é igual a `tokens_after` quando o L3 não dispara
(abaixo do threshold ou fallback). `tokens_after_l3` fica `null`
quando o L3 é pulado.

### Passo 2 — Persistir compressões em disco (opt-in)

**Novo campo de config** em `ModelConfig`:

```json
"logging": {
  "save_compressions": false,
  "log_dir": "~/.ntk/logs"
}
```

Ou controlar via env var `NTK_LOG_COMPRESSIONS=1` (mais simples, não
precisa migração de config).

Quando habilitado, o `POST /compress` grava um arquivo por chamada:

```
~/.ntk/logs/2026-04-15/<uuid>.json
{
  "ts":           "2026-04-15T03:42:18.123Z",
  "command":      "cargo test",
  "cwd":          "/home/user/project",
  "input":        "<stdin bruto do hook>",
  "after_l1":     "<output do L1>",
  "after_l2":     "<output do L2>",
  "after_l3":     "<output do L3 ou null>",
  "final":        "<o que foi retornado>",
  "tokens":       { "before": 3200, "l1": 1800, "l2": 1200, "l3": 280 },
  "latency_ms":   { "l1": 4, "l2": 12, "l3": 820 },
  "layer_used":   3
}
```

Proteções:
- Truncar `input` em `max_input_chars` (já é enforçado).
- Rotacionar / deletar arquivos com mais de 30 dias (sweep diário no
  startup do daemon, mesmo code path do `metrics.history_days`).

### Passo 3 — Biblioteca de fixtures

Adicionar em `bench/fixtures/` (diretório novo, não o `tests/fixtures/`
que é para testes unitários):

| Arquivo | Camada esperada | Tamanho alvo |
|---|---|---|
| `cargo_build_verbose.txt` | L1 (dedup de "Compiling X") | ~500 linhas |
| `cargo_test_com_falhas.txt` | L1 (remove passes, mantém falhas) | ~200 linhas |
| `tsc_errors_node_modules.txt` | L2 (encurtamento de paths) | ~150 linhas |
| `docker_logs_repetitivos.txt` | L1 (dedup massiva) | ~400 linhas |
| `log_longo_generico.txt` | L3 (semântico) | >300 tokens |
| `ja_curto.txt` | abaixo de `MinChars`, hook ignora | <500 chars |
| `git_diff_grande.txt` | L2 (token-aware) | ~500 linhas |
| `stack_trace_java.txt` | L3 (resumo estrutural) | ~60 linhas profundas |

Cada fixture tem um irmão `<nome>.meta.json`:

```json
{ "categoria": "build", "layer_esperada": 1, "ratio_minimo": 0.7 }
```

### Passo 4 — Script de microbench

**Arquivo:** `bench/replay.sh`

```bash
#!/bin/sh
# Dispara cada fixture contra o daemon vivo e grava uma linha CSV.
for fx in bench/fixtures/*.txt; do
  name=$(basename "$fx" .txt)
  payload=$(jq -Rs --arg cmd "$(cat $fx.meta.json | jq -r .command // \"unknown\")" \
    '{output: ., command: $cmd, cwd: "/"}' < "$fx")
  t0=$(date +%s%N)
  resp=$(curl -sf --max-time 120 -X POST \
    http://127.0.0.1:8765/compress \
    -H 'Content-Type: application/json' \
    -d "$payload")
  t1=$(date +%s%N)
  latency=$(( (t1 - t0) / 1000000 ))
  echo "$name,$resp,$latency" >> microbench.csv
done
```

Colunas do CSV: `fixture, bytes_in, tokens_before, tokens_after_l1,
tokens_after_l2, tokens_after_l3, tokens_after, layer_used, ratio,
latency_ms_total, latency_ms_l1, latency_ms_l2, latency_ms_l3`.

### Passo 5 — Prompt fixo de teste (`bench/prompts/baseline.md`)

O prompt precisa ser determinístico, disparar muitas ferramentas Bash
com outputs grandes, e não depender de estado de rede.

```markdown
Você está rodando dentro do repositório do NTK (`pwd` deve terminar
em `/ntk`). Rode os comandos **em ordem**, um por tool call, esperando
cada um terminar antes do próximo. NÃO resuma antes do passo 8.

1. `cargo build --release --verbose 2>&1 | head -400`
2. `cargo test --no-run --verbose 2>&1 | head -200`
3. `git log --stat --format=fuller -30 2>&1`
4. `find src -name "*.rs" -exec wc -l {} \; 2>&1 | sort -rn | head -30`
5. `cargo tree --edges normal --prefix depth 2>&1 | head -300`
6. `cargo clippy --release -- -W clippy::pedantic 2>&1 | head -300`
7. `ls -laR src/ 2>&1 | head -400`
8. Agora resuma: quantos arquivos Rust, LOC total, top-3 maiores
   módulos, e se o clippy reportou warnings.
```

Por que esse prompt:

- 7 chamadas Bash distintas — sample size suficiente por run.
- Mistura output estruturado (cargo) e não-estruturado (ls, find).
- Outputs variam de ~200 a ~1000+ linhas — cobre as três camadas.
- O passo 8 força o Claude a **ler** os outputs comprimidos (não só
  pular), então o caminho ponta-a-ponta é exercitado.
- Determinístico quando rodado no mesmo repo no mesmo commit.

### Passo 6 — Runner de sessão e parser

**Arquivo:** `bench/session.sh`

```sh
# Variante A: hook desabilitado
ntk init --uninstall
claude -p "$(cat bench/prompts/baseline.md)" \
  --output-format stream-json > transcripts/A.jsonl

# Variante B: hook habilitado + logging habilitado
ntk init -g
NTK_LOG_COMPRESSIONS=1 ntk start &
claude -p "$(cat bench/prompts/baseline.md)" \
  --output-format stream-json > transcripts/B.jsonl

# Parsear os dois
python3 bench/parse_transcript.py transcripts/A.jsonl > A.csv
python3 bench/parse_transcript.py transcripts/B.jsonl > B.csv
```

**`bench/parse_transcript.py`** lê o JSONL, soma os campos de
`message.usage` por turno, produz CSV com `turno, input_tokens,
cache_read_input_tokens, cache_creation_input_tokens, output_tokens,
total_tokens`.

Se o `claude -p` existe como modo não-interativo precisa ser
confirmado — ver *Perguntas em aberto* abaixo.

### Passo 7 — Gerador de relatório

**Arquivo:** `bench/report.py`

Lê `microbench.csv`, `A.csv`, `B.csv`, `~/.ntk/logs/...` e emite
`relatorio.md` com:

- **Tabela 1:** ratio de compressão por fixture (microbench).
- **Tabela 2:** totais A-vs-B da sessão (input tokens, output tokens,
  cache hits, custo estimado em USD — taxas configuráveis).
- **Tabela 3:** onde os tokens foram removidos — agrupado por
  categoria de fixture, mostrando contribuição % de L1/L2/L3.
- **Gráfico (ASCII):** histograma de ratios por camada.
- **Flags:** fixtures onde ratio < 20% (NTK quase não ajudou), ratios
  > 90% (NTK valeu a pena), ratios > 95% (verificar manualmente se
  houve perda de informação).

---

## Como rodar o teste de fato (fluxo sugerido)

```sh
# Pré-requisitos
cargo build --release                 # v0.2.24+
export NTK_LOG_COMPRESSIONS=1

# 1. Micro: validar a matemática do compressor
ntk start &
bash bench/replay.sh                  # escreve microbench.csv

# 2. Macro baseline: sem hook
ntk init --uninstall
# reiniciar Claude Code (hooks são carregados no início da sessão)
# colar manualmente o bench/prompts/baseline.md
# salvar transcript: cp ~/.claude/projects/<proj>/<session>.jsonl transcripts/A.jsonl

# 3. Macro com hook
ntk init -g
ntk start                             # sessão limpa pra métricas resetarem
# reiniciar Claude Code de novo
# colar o bench/prompts/baseline.md
# salvar transcript: cp ~/.claude/projects/<proj>/<session>.jsonl transcripts/B.jsonl

# 4. Relatório
python3 bench/report.py \
  --micro microbench.csv \
  --a transcripts/A.jsonl \
  --b transcripts/B.jsonl \
  --logs ~/.ntk/logs/ \
  --out relatorio.md
```

---

## Perguntas em aberto — precisam ser decididas antes de implementar

1. **Default do `save_compressions`** — desligado (privacidade,
   espaço em disco) ou ligado (zero atrito para benchmarking)?
   Recomendação: desligado, habilitar só via env var.

2. **Modo não-interativo do `claude -p`** — existe na versão
   instalada do Claude Code? Se não, o macrobench tem que ser rodado
   manualmente (usuário cola o prompt 2×, fecha o Claude entre
   runs). Verificar com `claude --help`.

3. **Reporte de custo** — só tokens, ou converter para USD usando
   taxas fixas do Sonnet 4.6 (`$3 / 1M input, $15 / 1M output`,
   cache hits a `$0.30 / 1M`)? As taxas mudam — se for USD, ler de
   um arquivo de config pra poder atualizar sem release de código.

4. **Escopo do relatório** — só CSV cru (usuário analisa), ou também
   markdown renderizado com conclusões? Se renderizado, decidir
   "NTK ajudou" vs "NTK atrapalhou" precisa de um threshold — qual
   é um corte justo? (Proposta: economia líquida < 10% dos
   tokens-before = "marginal"; < 0 = "overhead sem benefício".)

5. **O microbench deve quebrar o CI?** Se ratios regredirem abaixo
   do `ratio_minimo` no `<fixture>.meta.json`, virar um gate de
   release? Ou manter puramente informativo?

---

## Esforço esperado por passo

| Passo | O que muda | Esforço |
|---|---|---|
| 1. Métricas por camada na resposta | `src/server.rs`, `src/compressor/mod.rs` | ~1 h |
| 2. Persistência opt-in | Config + handler | ~1 h |
| 3. Biblioteca de fixtures | 8 `.txt` + `.meta.json` novos | ~1 h |
| 4. `replay.sh` + CSV | Shell script | ~30 min |
| 5. `baseline.md` | Texto | 15 min |
| 6. Runner de sessão + parser | Shell + Python | ~1,5 h |
| 7. Gerador de relatório | Python | ~2 h |
| **Total** | | **~7 h** de trabalho focado |

Os passos 1 e 2 são gate de tudo o resto (precisa dos dados
primeiro). Os passos 3-5 podem ser escritos em paralelo com 1-2.
Os passos 6-7 vêm por último.

---

## Ponto de decisão

Fechar as perguntas em aberto 1-4 (a pergunta 5 pode ser adiada),
depois a implementação segue na ordem acima. Cada passo é um commit /
PR separado para que regressões sejam bisectáveis.
