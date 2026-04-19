#!/usr/bin/env bash
# Creates the backlog of issues from the 2026-04-18 gap-analysis session.
# Idempotent-ish: re-running duplicates issues. Run once.
set -euo pipefail

export PATH="/c/Program Files/GitHub CLI:$PATH"

new() {
  local title="$1" labels="$2" body="$3"
  echo "-- $title"
  gh issue create --title "$title" --label "$labels" --body "$body" | tail -1
}

# =====================================================================
# P0 — Critical / foundational
# =====================================================================

new "feat(observability): \`test-compress --verbose\` com breakdown por camada" \
"feature,priority:P0,observability,testing" \
"## Problema

Hoje \`ntk test-compress\` devolve só o output final do pipeline. Quando uma
camada regride (ex: L3 piora o ratio em 5pp), não há como inspecionar o
input/output intermediário de cada etapa sem instrumentar código manualmente.

## Proposta

Adicionar flag \`--verbose\` que imprime, para cada camada:

- Tokens de entrada e saída + delta
- Latência da camada
- Regras aplicadas (ex: \`ansi_strip(4)\`, \`template_dedup(18→3)\`)
- Primeiras N linhas do output da camada

Formato esperado (stdout):

\`\`\`
┌─ L1 output (regex/filter) ─────── 12ms ────────
│ 1203 tokens (-49%), 312 lines
│ Applied: ansi_strip(4), template_dedup(18→3),
│          stack_trace_collapse(2 runs, 34 frames)
│ [first 20 lines of L1 output...]
└────────────────────────────────────────────────
\`\`\`

## Implementação

Cada \`LayerN::run()\` já retorna \`LayerOutput\` com \`output\`, \`latency\`,
\`applied_rules\`. Basta expor no CLI em vez de só agregar.

## Critério de pronto

- \`ntk test-compress fixture.txt --verbose\` imprime seções L1/L2/L4/L3 com métricas
- Sem \`--verbose\`, comportamento atual preservado
- Teste de integração que valida presença das seções"

new "fix(security): hook binds em 127.0.0.1 + header de auth + audit log" \
"fix,priority:P0,security" \
"## Problema

\`ntk-hook.sh\` vê todo output de todo comando Bash do usuário
(\`env\`, \`cat ~/.ssh/id_rsa\`, etc). Se a porta 8765 for exposta por engano
(bind 0.0.0.0, port-forward VSCode, WSL bridge), qualquer processo na rede
pode consumir dados sensíveis.

## Proposta

- **Bind explícito em 127.0.0.1** (não \`0.0.0.0\`) com validação no startup
- **Header fixo** \`X-NTK-Token\` no hook + daemon; token gerado em \`ntk init\`
  e armazenado em \`~/.ntk/.token\` (mode 0600)
- **Audit log** opcional (\`config.security.audit_log = true\`) com timestamp +
  tool_name + hash do output (não o output inteiro)

## Critério de pronto

- Teste que confirma \`bind\` rejeita endereço não-loopback
- Teste que request sem header ou com token inválido retorna 401
- Doc em README sobre o modelo de ameaça"

new "chore(ci): adicionar cargo-deny para validação de licenças transitivas" \
"chore,priority:P0,ci,security" \
"## Problema

Licença do NTK foi migrada Apache-2.0 → MIT, mas dependências transitivas
(\`candle\`, \`tokenizers\`, \`tiktoken-rs\`, etc) têm licenças próprias. Sem
validação, podemos absorver GPL/LGPL sem perceber.

## Proposta

- Adicionar \`cargo-deny\` ao job de CI (\`.github/workflows/ci.yml\`)
- Criar \`deny.toml\` com allowlist de licenças permitidas
  (MIT, Apache-2.0, BSD-3-Clause, ISC, Unicode-DFS-2016, Zlib)
- Bloquear merge se qualquer dep adicionar licença fora da lista

## Critério de pronto

- \`cargo deny check licenses\` passa no master
- Job \`license-check\` gating de PR
- CONTRIBUTING.md atualizado com a regra"

# =====================================================================
# P1 — High value
# =====================================================================

new "feat(L3): cache de inferência em SQLite (hash input → output)" \
"feature,priority:P1,layer:L3,performance" \
"## Problema

\`cargo test\` rodado 10× no mesmo branch produz o mesmo output → L3 é
chamado 10× com a mesma entrada. Em GPU é ~50ms × 10 = 500ms desperdiçados.
Em CPU é segundos.

## Proposta

- Tabela \`l3_cache(hash TEXT PRIMARY KEY, output TEXT, model TEXT, created_at)\`
- Chave: \`sha256(l2_output + l4_context + model + prompt_format)\`
- Lookup antes de chamar backend; se hit, pula inferência
- TTL configurável (\`config.l3_cache.ttl_days = 7\`)
- Métricas: hit rate exposto em \`ntk metrics\`

## Critério de pronto

- 2ª chamada idêntica retorna em <5ms
- Teste de integração com 2 requests iguais, valida cache hit
- \`ntk metrics\` mostra \`l3_cache_hit_rate\`"

new "feat(L4): bench sistemático com/sem context prefix para validar impacto" \
"feature,priority:P1,layer:L4,testing" \
"## Problema

L4 injeta contexto antes de L3, mas **nunca foi medido sistematicamente**
se isso melhora a qualidade/ratio da compressão. O formato default
(\`Prefix\`) foi escolhido por palpite.

## Proposta

Estender \`bench/prompt_formats.ps1\` para rodar cada fixture 2×:
1. Sem contexto
2. Com contexto extraído de um transcript sintético realista

Métricas comparadas:
- Ratio médio final
- Preservação de sinal de erro (regex check sobre output)
- Latência adicional de L4

Se L4 não ganha >2pp em ratio OU piora preservação de erro, **L4 deve ser
desabilitado por default**.

## Critério de pronto

- CSV com coluna \`context_enabled\`
- Relatório markdown com conclusão
- Se ganhar: documentar. Se não: issue #remover-L4"

new "feat(testing): snapshots insta por camada (L1/L2/L3 isolados)" \
"feature,priority:P1,testing,observability" \
"## Problema

Hoje snapshots cobrem pipeline completo. Quando ratio final cai 3pp, não há
como saber rapidamente qual camada regrediu — é preciso instrumentar e rerun.

## Proposta

Expandir \`tests/snapshots/\` com arquivos por camada e fixture:

\`\`\`
tests/snapshots/layer1__stack_trace_python.snap
tests/snapshots/layer2__stack_trace_python.snap
tests/snapshots/layer3__stack_trace_python.snap
\`\`\`

Teste parametrizado itera fixtures × camadas e grava snapshot. \`cargo insta\`
aponta exatamente onde regrediu.

## Critério de pronto

- Snapshot por camada para todas fixtures em \`bench/fixtures/\`
- Doc em CONTRIBUTING.md sobre \`cargo insta review\`"

new "feat(observability): comando \`ntk diff <fixture> --layer\` side-by-side" \
"feature,priority:P1,observability" \
"## Problema

Para entender o que uma camada fez, hoje é preciso ler o input e o output
separadamente e comparar de cabeça.

## Proposta

\`\`\`
ntk diff fixture.txt --layer l1
# imprime input | output lado a lado, destaca linhas modificadas
\`\`\`

Flags: \`--layer {l1,l2,l3,all}\`, \`--no-color\`, \`--width <n>\`.

Reaproveitar bibliotecas existentes: \`similar\`, \`console\`.

## Critério de pronto

- Diff colorido para as 3 camadas (L3 opcional se daemon rodando)
- Teste snapshot do formato"

new "feat(L3): streaming de tokens (reduz p95 e permite cancelamento)" \
"feature,priority:P1,layer:L3,performance" \
"## Problema

Backend L3 é request/response bloqueante. Em outputs grandes, o daemon
segura a resposta inteira antes de devolver ao hook. Não há como cancelar
inferência se o usuário interromper a sessão.

## Proposta

- Usar streaming da API Ollama / Candle / llama.cpp
- Chunking de tokens no response do \`/compress\`
- Token budget: parar inferência se exceder tempo máximo configurável

## Critério de pronto

- p95 reduz em pelo menos 20% em fixtures grandes
- Cancel via drop do client funciona (teste com \`tokio::time::timeout\`)"

new "feat(L3): fallback automático entre backends (Ollama → Candle → L1+L2)" \
"feature,priority:P1,layer:L3" \
"## Problema

Se Ollama morre mid-session, NTK cai imediatamente pra L1+L2 sem tentar
Candle (que pode estar disponível na mesma máquina). All-or-nothing.

## Proposta

Cadeia de fallback configurável:

\`\`\`json
\"model\": {
  \"backend_chain\": [\"ollama\", \"candle\", \"llama_cpp\"],
  \"fallback_threshold_ms\": 2000
}
\`\`\`

Primeiro backend que responde < threshold é usado; se timeout ou erro,
tenta o próximo. Se todos falham, L1+L2 only.

## Critério de pronto

- Teste com \`wiremock\` simulando Ollama down → Candle up → response OK
- Métrica \`l3_backend_used\` em \`ntk metrics\`"

# =====================================================================
# P2 — Medium value
# =====================================================================

new "feat(L4): múltiplas mensagens de intent com decay temporal" \
"feature,priority:P2,layer:L4" \
"## Problema

L4 extrai só a última mensagem do usuário. Em sessões longas onde o intent
é \"ainda estou debugando o mesmo bug há 10 mensagens\", o contexto real se
perde.

## Proposta

- Extrair até N últimas mensagens de user (default: 3)
- Peso decrescente por idade (mensagem de 3h atrás pesa menos)
- Intent combinado permanece ≤ \`MAX_INTENT_CHARS\` (500)

## Critério de pronto

- Testes cobrindo cenários: única msg, 3 msgs recentes, msgs antigas
- Bench comparando ratio com 1 vs 3 msgs"

new "feat(observability): \`ntk tail\` / \`ntk logs --follow\`" \
"feature,priority:P2,observability" \
"## Problema

Pra saber o que o daemon está comprimindo em tempo real, hoje precisa
abrir o SQLite manualmente. UX ruim pra debug interativo.

## Proposta

\`\`\`
ntk tail              # últimas 10 compressões + follow
ntk tail --since 1h
ntk tail --command cargo
\`\`\`

Output compacto por linha: \`HH:MM:SS | cargo test | 2341→304 (87%) | 243ms | L1+L2+L3\`.

## Critério de pronto

- \`--follow\` stream em tempo real via SSE ou polling SQLite
- Filtros por comando, tempo, layer"

new "feat(observability): dashboard HTML local em /dashboard" \
"feature,priority:P2,observability" \
"## Problema

\`ntk graph\` é one-shot stdout. Pra visualização ao longo do dia, queremos
algo contínuo.

## Proposta

Daemon serve \`GET /dashboard\` com HTML estático + JS que consome
\`/metrics\` periodicamente.

Widgets: economia por hora (chart), layer distribution (pie), top commands
(bar), p50/p95 latência.

## Critério de pronto

- Acessível em \`http://127.0.0.1:8765/dashboard\`
- Sem dependência externa (CSS/JS inline ou vendor em \`docs/vendor/\`)
- Atualiza a cada 5s"

new "feat(platform): hook para editores adicionais (Cursor, Continue, Aider, Zed)" \
"feature,priority:P2,platform" \
"## Problema

\`ntk init\` só conhece Claude Code e OpenCode. Usuários de Cursor, Aider,
Continue, Zed têm que instrumentar manualmente.

## Proposta

- Detectar editores instalados (\`dirs::config_dir()\` + paths conhecidos)
- Flag \`--editor <cursor|continue|aider|zed>\`
- Cada editor tem seu próprio formato de hook/extension; adicionar templates
  em \`scripts/editors/\`

## Critério de pronto

- Pelo menos Cursor e Continue funcionando end-to-end
- Doc em README com matriz de editores suportados
- CONTRIBUTING.md explicando como adicionar novo editor"

new "fix(config): \`.ntk.json\` per-project merge consistente com global" \
"fix,priority:P2,config" \
"## Problema

README documenta \`.ntk.json\` sobrescrevendo \`~/.ntk/config.json\`, mas
\`config.rs\` não faz merge deep consistentemente. Campo omitido no local
às vezes zera global em vez de herdar.

## Proposta

- Merge recursivo (semver-style) com testes explícitos
- Exemplos no README com 3 cenários: override total, parcial, herdar tudo
- Campos não setados: preservam global

## Critério de pronto

- 6+ testes de merge cobrindo campos aninhados
- Exemplo funcional em \`examples/project-config/\`"

new "feat(bench): \`ntk bench --submit\` gera JSON padronizado para issues" \
"feature,priority:P2,testing,performance" \
"## Problema

CONTRIBUTING.md pede benchmarks em hardware alheio, mas não há formato
padronizado. PRs chegam com \"testei no meu M2, parece rápido\" — não dá
pra comparar.

## Proposta

\`\`\`
ntk bench --submit
# → ~/.ntk/bench-report-<timestamp>.json
# → abre URL para anexar em issue template
\`\`\`

JSON inclui: hardware (CPU/GPU), OS, backend detectado, latências p50/p95/p99
por camada, ratios por fixture, versão NTK.

## Critério de pronto

- Template \`.github/ISSUE_TEMPLATE/bench-report.md\` consome o JSON
- Script de agregação em \`scripts/aggregate-benches.ps1\`"

new "fix(robustness): fuzz test com input binário/não-UTF8" \
"fix,priority:P2,testing" \
"## Problema

Alguém vai rodar \`cat binary.bin\` e mandar 2MB de bytes não-UTF8 pro daemon.
\`tiktoken-rs\` pode panicar em input malformado.

## Proposta

- Target \`cargo-fuzz\` para \`layer1::filter\` e \`layer2::compress\`
- Input: bytes aleatórios (\`Arbitrary\` ou raw)
- Invariante: nenhum panic, sempre retorna \`Ok\`

## Critério de pronto

- \`cargo fuzz run layer1_filter -- -max_total_time=60\` passa limpo
- CI opcional nightly job roda fuzz por 10min"

# =====================================================================
# P3 — Low / nice-to-have
# =====================================================================

new "chore(L2): suporte a tokenizers além de cl100k_base" \
"chore,priority:P3,layer:L2" \
"## Problema

L2 conta tokens com \`cl100k_base\` (GPT-3.5/4 / Claude antigo). Claude 3.5
Sonnet e GPT-4o usam \`o200k_base\`. Contagem enviesada ~5-10%.

## Proposta

- Config \`model.tokenizer\` com enum: \`cl100k\`, \`o200k\`, \`claude3\`
- Auto-detect opcional baseado em header do cliente
- Fallback para \`cl100k\` se desconhecido

## Critério de pronto

- Suporte ≥ 2 tokenizers
- Teste comparando counts em fixture conhecida"

new "chore(L3): rodar prompt_formats.ps1 sistematicamente e escolher default" \
"chore,priority:P3,layer:L3,testing" \
"## Problema

4 variantes de \`PromptFormat\` (Prefix/XmlWrap/Goal/Json) e o default
atual é palpite. Sem evidência, não há justificativa para o #[default].

## Proposta

- Rodar \`bench/prompt_formats.ps1\` em todas fixtures L3-triggering
- Comparar ratio médio + preservação de erro
- Documentar resultado em \`docs/prompt-format-ablation.md\`
- Se vencedor ≠ default atual: trocar e abrir PR citando os números

## Critério de pronto

- Relatório markdown commitado
- #[default] refletindo resultado"

new "chore: decidir destino da telemetria (ativar endpoint ou remover código)" \
"chore,priority:P3" \
"## Problema

\`src/telemetry.rs\` existe, mas não há endpoint rodando pra receber.
Código que nunca é exercido vira dívida — regressões passam despercebidas.

## Proposta

**Opção A:** subir endpoint (Cloudflare Worker + KV) e manter código ativo.
**Opção B:** remover \`telemetry.rs\` e suas refs, documentar que pode
voltar no futuro.

## Critério de pronto

- Decisão documentada em \`docs/telemetry-decision.md\`
- Código coerente com a decisão"

new "feat(metrics): \`ntk prune --older-than 30d\` rotação de SQLite" \
"feature,priority:P3,config" \
"## Problema

Tabela de métricas cresce indefinidamente. Usuários de longa data verão
SQLite de GB.

## Proposta

- Comando \`ntk prune --older-than <duration>\`
- Opção \`config.metrics.auto_prune_days\` para agendamento automático
- \`VACUUM\` após delete para liberar disco

## Critério de pronto

- Teste com 10k linhas, prune, valida tamanho do arquivo
- Doc em README"

new "fix(L1): apertar min_ratio PHP e Python (classificadores mais agressivos)" \
"fix,priority:P3,layer:L1,testing" \
"## Problema

\`min_ratio\` PHP = 20%, Python = 30-45%. Outros ficam em 50-70%. Há espaço
pra classificador mais agressivo sem perder sinal.

## Proposta

- Auditar fixtures PHP/Python: quais frames ainda escapam da coleta
- Estender tabelas em \`is_framework_frame\` (ver \`stack-trace-classifier.md\`)
- Apertar \`min_ratio\` em \`.meta.json\` após confirmação
- Manter invariante #1 (zero perda de sinal de erro)

## Critério de pronto

- PHP ≥ 40%, Python ≥ 50%
- Proptest \`prop_error_signals_are_preserved\` passa"

new "chore(ci): determinismo do bench (runner dedicado ou threshold com margem)" \
"chore,priority:P3,ci,performance,testing" \
"## Problema

\`cargo bench\` (\`criterion\`) não é determinístico entre GitHub runners.
CI pode ficar flaky se tratarmos bench como contrato.

## Proposta

**Opção A:** runner self-hosted dedicado para bench (hardware fixo).
**Opção B:** threshold com margem de ±15% + só fail em regressão consistente
em 3 runs consecutivas.

## Critério de pronto

- Decisão documentada
- Bench não produz falso positivo em 10 runs seguidas do master"

# =====================================================================
# Context Linter Spec — POC (from Planning)
# =====================================================================

new "rfc: RFC-0001 — Context Linter Spec (meta-issue)" \
"rfc,priority:P2" \
"## Visão

Extrair as regras de compressão do NTK para um **formato declarativo aberto**
(YAML/JSON) que qualquer agente LLM (Cursor, Cline, Aider, OpenCode) possa
consumir. NTK (Rust) vira a implementação de referência.

Analogia: **ESLint é ao JavaScript o que NTK-Spec seria ao output de LLM agents.**

## Validação em 3 etapas

Essa issue é **meta-tracking**. As sub-issues abaixo representam os passos de
validação baratos e reversíveis antes de qualquer commitment público.

- [ ] Etapa 1 — POC interna: migrar uma família de regras para formato declarativo (#TBD)
- [ ] Etapa 2 — RFC público com schema + 3 exemplos + 30 dias de coleta (#TBD)
- [ ] Etapa 3 — Binding de referência em segundo agente (#TBD)

## Critério de kill-switch

- Etapa 1 com overhead > 20% vs código hardcoded → descartar
- Etapa 2 com < 3 comentários substantivos externos → engavetar
- Etapa 3 com segundo agente precisando modificar Rust → schema v2"

new "feat(rfc): POC Etapa 1 — migrar stack-trace Python para YAML declarativo" \
"feature,rfc,priority:P2,layer:L1,testing" \
"## Objetivo

Reescrever **uma** família de regras existente (stack-trace Python) como
arquivo declarativo carregado em runtime. Medir custo e legibilidade.

## Formato hipotético

\`\`\`yaml
rule: stack-trace-python-framework
pattern:
  kind: frame-run
  classifier: starts_with
  values:
    - \"  File \\\"/site-packages/\"
    - \"  File \\\"/gunicorn/\"
    - \"  File \\\"/asgiref/\"
transform:
  kind: collapse-run
  min_run: 3
  replacement: \"[{n} framework frames omitted]\"
severity: lossy-safe
intent_scope:
  preserve_on: [\"debug\", \"fix-failing-test\"]
  apply_on: [\"run-tests\", \"ci-check\"]
\`\`\`

## Métricas obrigatórias

- **Overhead**: < 10% vs código hardcoded em \`cargo bench layer1\`. Se > 20%, kill.
- **Legibilidade**: contribuidor consegue adicionar regra Ruby lendo só o YAML?
  (pedir review externo de 1-2 pessoas)

## Critério de pronto

- Loader YAML em \`src/compressor/spec_loader.rs\`
- \`tests/fixtures/rules/python.yaml\` funcionando
- Bench comparativo commitado em \`docs/spec-poc-bench.md\`
- Decisão go/no-go para Etapa 2"

new "rfc: POC Etapa 2 — publicar RFC-0001 draft e abrir comentários (30 dias)" \
"rfc,priority:P3" \
"## Pré-requisito

**NÃO abrir esta etapa antes de:**
- Etapa 1 concluída com overhead aceitável
- Pelo menos ~100 stars OU 5 contribuidores externos ativos

Sem tração, RFC vira documento morto.

## Escopo

Publicar \`docs/rfcs/0001-context-linter-spec.md\` no repo com:
- Schema formal completo (frontmatter JSON Schema)
- 3 exemplos: stack-trace, docker logs, tsc output
- Questões abertas explícitas (intent scope, transform power, severity)
- Prazo de 30 dias para comentários via issues com label \`rfc-0001\`

## Critério de sucesso

≥ 3 comentários substantivos de pessoas fora do círculo inicial. Menos que
isso = tese não ressoa, volta ao foco atual.

## Critério de pronto

- RFC publicado
- Label \`rfc-0001\` criada
- Thread no Discussions do repo
- Deadline agendado em calendário"

new "feat(rfc): POC Etapa 3 — binding de referência em segundo agente" \
"feature,rfc,priority:P3,platform" \
"## Pré-requisito

Etapa 2 aprovada com ≥ 3 comentários substantivos.

## Escopo

Escolher **um** agente open-source além do Claude Code (sugestão: Continue
ou Aider) e portar o hook consumindo a spec YAML.

## Critério de sucesso

O segundo agente consegue rodar as mesmas regras **sem modificar o código Rust do NTK**.
Se precisar modificar → schema v2.

## Critério de pronto

- PR no repo do agente escolhido OU fork com integração funcional
- Doc com passos reproduzíveis
- Relatório em \`docs/spec-second-binding.md\`"

echo
echo "All issues created."
