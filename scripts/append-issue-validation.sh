#!/usr/bin/env bash
# Appends a "Validação (2026-04-18)" section to each issue body.
# Idempotent: checks if the section already exists before appending.
set -euo pipefail

export PATH="/c/Program Files/GitHub CLI:$PATH"

append() {
  local num="$1" validation="$2"
  local current
  current="$(gh issue view "$num" --json body -q .body)"
  if printf '%s' "$current" | grep -q "## Validação (2026-04-18)"; then
    echo "#$num — já validada, pulando"
    return
  fi
  local new_body="$current

---

## Validação (2026-04-18)

$validation"
  gh issue edit "$num" --body "$new_body" > /dev/null
  echo "#$num — atualizada"
}

append 1 "**Estado atual:** \`TestCompress\` em \`src/main.rs:84-99\` já tem \`with_l3 / context / l4_format / daemon_url\`, mas **não expõe \`--verbose\`**. \`CompressResponse\` em \`src/server.rs:67-94\` já carrega \`LayerLatency { l1, l2, l3 }\`.

**Premissa procede?** **Parcial.** A infra de latências por camada existe; falta (a) flag \`--verbose\`, (b) captura das regras aplicadas (não retornadas hoje pelas camadas), (c) print dos outputs intermediários.

**Ajuste de escopo:** além do CLI, precisa instrumentar L1/L2 para retornar \`applied_rules: Vec<String>\` em \`LayerOutput\`. Senão \"Applied: ansi_strip(4)...\" fica vazio."

append 2 "**Estado atual:** bind em \`src/main.rs:396\` via \`TcpListener\` — não há validação explícita de loopback. Router em \`src/server.rs:115-119\` expõe \`/compress /metrics /records /health /state\` **sem middleware de auth** e **sem header check**. Não há audit log (existe só evento de telemetria em \`src/server.rs:330\`).

**Premissa procede?** **Sim, integralmente.** Os três gaps apontados (bind, auth, audit) estão todos em aberto.

**Nota de risco:** severidade P0 confirmada — qualquer port-forward acidental expõe output de todo Bash."

append 3 "**Estado atual:** não existe \`deny.toml\` na raiz. \`.github/workflows/ci.yml\` não faz referência a \`cargo deny\`.

**Premissa procede?** **Sim.** Nenhum dos pontos da proposta está implementado.

**Nota:** existe \`.cargo/audit.toml\` (RUSTSEC ignores) — é auditoria de vulns, não licenças; são complementares."

append 4 "**Estado atual:** esquema SQLite em \`src/metrics.rs:162-172\` cobre só \`compression_records\` (id, command, tokens, latency...). Nenhuma tabela de cache.

**Premissa procede?** **Sim.** Não há cache L3 implementado.

**Sugestão:** pode coexistir na mesma conexão \`sqlx\` — basta adicionar migração \`002_l3_cache.sql\`."

append 5 "**Estado atual:** \`bench/prompt_formats.ps1:63\` já envia campo \`context\` no payload POST. Em \`bench/prompt_formats.ps1:28\` o contexto é parametrizável. CSV gerado tem colunas \`format, fixture, tokens, ratio, layer, latency_ms, error\` — **sem coluna \`context_enabled\`**.

**Premissa procede?** **Parcial.** Suporte técnico existe; falta rodar o experimento A/B e registrar a dimensão no CSV.

**Escopo restante:** adicionar flag \`-CompareContext\` no script que roda 2× cada fixture e escreve ambas linhas com \`context_enabled=true/false\`."

append 6 "**Estado atual:** \`tests/integration/snapshots/\` contém 6 snapshots, todos no padrão \`snapshot_tests__<fixture>.snap\` (pipeline completo).

**Premissa procede?** **Sim.** Não há snapshots por camada.

**Sugestão:** adicionar módulo \`tests/integration/snapshots_per_layer.rs\` com \`#[rstest]\` parametrizando \`(fixture, layer)\` — reaproveita mesmos fixtures de bench."

append 7 "**Estado atual:** \`src/main.rs:22-129\` enumera \`Init, Start, Stop, Status, Metrics, Graph, Gain, History, Config, TestCompress, Model, Dashboard, Discover, Test, Bench\`. Nenhum \`Diff\`.

**Premissa procede?** **Sim.** Subcomando não existe.

**Observação:** depende de #1 (\`--verbose\` retornando outputs intermediários) para ter fonte do L1/L2 sem reimplementar."

append 8 "**Estado atual:** \`src/compressor/layer3_backend.rs:29-33\` define \`enum BackendKind { Ollama, Candle, LlamaCpp }\` com dispatch via match. Nenhum método \`stream\` — interface é request/response.

**Premissa procede?** **Sim.** Streaming não implementado em nenhum backend.

**Escopo real:** precisa converter \`BackendKind\` em trait async com método \`compress_stream()\` retornando \`impl Stream<Item=String>\`. Mudança arquitetural não-trivial."

append 9 "**Estado atual:** \`src/config.rs:93\` tem \`fallback_to_layer1_on_timeout: bool\`. Fallback atual é binário (backend ativo → L1+L2). Não há cadeia multi-backend.

**Premissa procede?** **Parcial.** Existe o conceito de fallback, mas sem cadeia configurável.

**Escopo ajustado:** substituir \`provider: String\` único por \`backend_chain: Vec<String>\`; manter \`provider\` como alias deprecated para migração."

append 10 "**Estado atual:** \`src/compressor/layer4_context.rs:70\` — comentário explícito \"Walk the transcript in reverse — the most recent user message wins.\" Retorna 1 mensagem. Truncagem a 500 chars em \`layer4_context.rs:170\`.

**Premissa procede?** **Sim.** Extração é single-message, sem decay.

**Nota:** manter \`MAX_INTENT_CHARS=500\` total quando combinar múltiplas msgs para não explodir o prefix."

append 11 "**Estado atual:** enum \`Command\` em \`src/main.rs:22-129\` não tem \`Tail\` nem \`Logs\`.

**Premissa procede?** **Sim.** Não existe.

**Dependência:** requer endpoint de streaming no daemon (SSE ou WebSocket) — aproveitar se #8 trouxer infra de stream."

append 12 "**Estado atual:** router em \`src/server.rs:113-121\` não expõe \`/dashboard\`. Existe módulo \`src/output/dashboard.rs\` mas é para TUI (stdout), não para servir HTML.

**Premissa procede?** **Sim.** Rota HTTP não existe.

**Sugestão:** reaproveitar o cálculo de widgets que já está em \`dashboard.rs\` — só mudar o renderer para HTML + servir em rota nova."

append 13 "**Estado atual:** \`src/installer.rs:11-14\` define \`enum EditorTarget { ClaudeCode, OpenCode }\`. \`scripts/\` tem só \`ntk-hook.sh\` e \`ntk-hook.ps1\`, sem subpasta \`editors/\`. \`CONTRIBUTING.md:36-39\` já lista Cursor/Aider/Zed/Continue/Windsurf como futuros ports.

**Premissa procede?** **Sim.** Apenas 2 editores cobertos.

**Escopo incremental:** cada editor pode ser PR separado. Começar por Cursor (mesmo formato JSON que Claude Code) e Aider (hook via YAML config)."

append 14 "**Estado atual:** \`src/config.rs:245-248\` chama \`load_file_or_default()\` seguido de \`merge_local()\`. \`src/config.rs:289-299\` implementa \`merge_json()\` recursivo (**deep merge**).

**Premissa procede?** **Parcial / possivelmente não.** Deep merge já existe. Pode haver regressão específica; precisa reproduzir o cenário descrito na issue antes de implementar.

**Ação sugerida:** antes de codar, adicionar teste com o cenário \"campo omitido no local herda global\" e confirmar se quebra. Se passar, fechar issue como resolvida."

append 15 "**Estado atual:** \`src/main.rs:121-127\` \`Bench { runs, l3 }\` — sem \`--submit\`.

**Premissa procede?** **Sim.** Flag não existe.

**Nota:** pode reaproveitar JSON gerado por \`bench/replay.ps1\` — só padronizar campos e escrever no stdout do \`ntk bench --submit\`."

append 16 "**Estado atual:** pasta \`fuzz/\` não existe na raiz; nenhum \`Cargo.toml\` de fuzz target.

**Premissa procede?** **Sim.** \`cargo-fuzz\` não configurado.

**Escopo:** \`cargo fuzz init\` + 2 targets (\`layer1_filter\`, \`layer2_compress\`). CI nightly opcional; local-only aceitável no curto prazo."

append 17 "**Estado atual:** \`src/compressor/layer2_tokenizer.rs:5\` instancia só \`cl100k_base()\`. Nenhuma referência a \`o200k_base\` ou \`claude\`.

**Premissa procede?** **Sim.** Suporte único.

**Nota:** \`tiktoken-rs\` já expõe \`o200k_base()\`; upgrade é mecânico, o difícil é definir o critério de seleção (header? config?)."

append 18 "**Estado atual:** \`docs/\` contém \`app.js, index.html, install.ps1/sh, plano-de-testes.md, testing-plan.md, style.css\`. Nenhum \`prompt-format-ablation.md\`.

**Premissa procede?** **Sim.** Documento não existe.

**Pré-requisito:** #5 precisa entregar o CSV com context_enabled para que este doc tenha números."

append 19 "**Estado atual:** \`src/telemetry.rs:244\` define \`TELEMETRY_ENDPOINT = \"https://telemetry.ntk.dev/v1/ping\"\`. Opt-out via \`NTK_TELEMETRY_DISABLED\` env (\`src/telemetry.rs:40-45\`).

**Premissa procede?** **Sim.** Código existe e aponta para endpoint externo. Se o domínio \`telemetry.ntk.dev\` não resolve hoje, é fire-and-forget silencioso — funcional mas sem coleta.

**Decisão pendente:** verificar se \`telemetry.ntk.dev\` está de pé. Se não: remover ou apontar para Cloudflare Worker real."

append 20 "**Estado atual:** enum \`Command\` em \`src/main.rs:22-129\` sem \`Prune\`.

**Premissa procede?** **Sim.** Não existe.

**Nota:** \`sqlx\` + \`DELETE WHERE created_at < ?\` + \`VACUUM\` é trivial; UX do comando é o principal a decidir (\`--older-than 30d\` vs \`--before 2026-01-01\`)."

append 21 "**Estado atual confirmado:** \`bench/fixtures/php_symfony_trace.meta.json:4\` tem \`min_ratio: 0.35\`; \`bench/fixtures/python_django_trace.meta.json:4\` tem \`min_ratio: 0.30\`.

**Premissa procede?** **Sim.** Margens realmente baixas vs outras linguagens (Java/Node em 0.5-0.7).

**Nota:** a issue original mencionava PHP=20%; número real é 35%. Ajustar meta da issue: PHP ≥ 45%, Python ≥ 45% parecem alvos mais honestos até auditoria empírica."

append 22 "**Estado atual:** \`.github/workflows/ci.yml:149\` — \`cargo bench -- --sample-size 10\`, resultado via \`tail -20\`. Sem threshold, sem regressão gate.

**Premissa procede?** **Sim.** Bench roda no CI mas não é contrato.

**Nota:** com \`sample-size=10\` a variância é alta; threshold ±15% (opção B da issue) é realista sem runner dedicado."

append 23 "**Estado atual:** \`README.md:21\` tem \"This project is an open initiative — it needs your help to evolve.\" \`CONTRIBUTING.md:3\` confirma mesma frase.

**Premissa procede?** **Sim, tração atual = 0.** Call-to-action existe; métrica de sucesso (stars, contribuidores) ainda não foi atingida.

**Ação de tracking:** esta issue é meta — só muda para \"unblocked\" quando pelo menos ~100 stars ou 5 contribuidores externos ativos."

append 24 "**Estado atual:** \`src/compressor/spec_loader.rs\` não existe. Nenhum loader declarativo implementado.

**Premissa procede?** **Sim.** Greenfield.

**Dependência lógica:** para a medição de overhead (<10% vs hardcoded) fazer sentido, o bench \`bench/replay.ps1\` precisa rodar ambas versões lado a lado — definir \`NTK_SPEC_RULES=1\` como flag experimental."

append 25 "**Estado atual:** \`docs/rfcs/\` não existe. Nenhum RFC-0001 publicado.

**Premissa procede?** **Sim.** Estrutura ausente.

**Gate:** manter \"do not open until Etapa 1 passa + tração mínima\" — validar antes de publicar."

append 26 "**Estado atual:** sem código de integração com Continue/Aider/Cursor. \`CONTRIBUTING.md:36-39\` lista só como \"future ports\". Nenhum exemplo em \`examples/\`.

**Premissa procede?** **Sim.** Greenfield.

**Sugestão de ordem:** Continue (extensão VSCode com hooks JS) é mais fácil de instrumentar que Aider (CLI Python com flow diferente). Começar por Continue se esta etapa for ativada."

echo
echo "Validação adicionada às 26 issues."
