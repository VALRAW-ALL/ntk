# Rule: Implementation Gate — segurança, memória, qualidade, fidelidade à issue

Applies to: **every code change** in `src/`, `tests/`, `scripts/`, `bench/`, or
`.github/workflows/`. Não é opcional; é o checklist mínimo antes de abrir PR
ou marcar uma issue como resolvida.

## Quando esta regra dispara

- Qualquer implementação de feature (`feature`, `rfc`)
- Qualquer correção (`fix`, `security`)
- Qualquer refactor que mude comportamento observável
- **Não** dispara para docs puros, renomes triviais sem semantic change,
  comentários isolados.

## Gate 1 — Segurança

Para cada mudança, responder explicitamente cada item. Se a resposta for
\"não se aplica\", justificar.

- [ ] **Input não confiável tratado?** Todo dado vindo de `stdin`, HTTP,
      arquivo de disco ou env var é validado antes de indexar, alocar, ou
      passar a `unsafe`. Path traversal, SSRF (ex: `config.ollama_url`),
      prompt injection em L3, JSON malformado em transcripts — tudo
      coberto.
- [ ] **Superfície de rede explícita?** Qualquer `bind`/`listen` usa
      `127.0.0.1` por default. Rotas novas exigem middleware de auth
      (header `X-NTK-Token`) salvo justificativa.
- [ ] **Sem `unwrap()` / `expect()` / `panic!()` em `src/`.** Tests (`tests/`)
      podem usar. Ver `clippy-gate.md`.
- [ ] **Secrets nunca em logs.** Tokens, paths absolutos de home, conteúdo
      completo de transcript — redigir ou hashear antes de `tracing::info`.
- [ ] **Dependência nova auditada?** Se `Cargo.toml` mudou, rodar
      `cargo audit` e `cargo deny check licenses` (quando #3 entregar).
      Licenças copyleft (GPL/LGPL/AGPL) **rejeitadas**.
- [ ] **`unsafe` comentado.** Todo bloco `unsafe` carrega uma `// SAFETY:`
      explicando a invariante mantida.

## Gate 2 — Memória & overhead

- [ ] **Alocações proporcionais ao input?** Transcripts crescem sem limite;
      parsing sempre line-by-line (streaming), nunca `read_to_string` da
      sessão inteira. Mesmo vale para outputs de Bash grandes.
- [ ] **Limites explícitos em estruturas que crescem?** Cache, vec, hashmap
      que podem encher em long-running daemon têm `max_entries` ou TTL.
      \"Funciona no benchmark\" ≠ \"funciona após 8h de sessão\".
- [ ] **Regex compilada uma vez.** Usar `once_cell::Lazy<Regex>` em vez de
      `Regex::new` no hot path.
- [ ] **Budget de latência respeitado?** L1+L2 em 10k linhas < 50ms; L3
      respeita `inference_threshold_tokens`. Se a mudança toca hot path,
      rodar `cargo bench` comparando com baseline antes/depois.
- [ ] **Overflow aritmético protegido.** `usize`/`u32` com dados externos
      usa `saturating_add/sub/mul` ou `checked_*`. Ver `clippy-gate.md`.
- [ ] **Async não bloqueante?** Dentro de `tokio::spawn` / handler axum,
      nunca `std::fs`, nunca `sqlx::blocking`, nunca `std::thread::sleep`.
      Usar `tokio::fs`, `sqlx::sqlite::SqlitePool`, `tokio::time::sleep`.

## Gate 3 — Boas práticas

- [ ] **Função faz uma coisa.** Se passou de ~50 linhas ou tem mais de 3
      níveis de indentação, quebrar antes de commitar.
- [ ] **Nomes em inglês**, descritivos, sem abreviação obscura
      (`filter_stack_frames`, não `flt_sf`).
- [ ] **Sem magic numbers.** Constantes nomeadas no topo do módulo
      (`const MAX_INTENT_CHARS: usize = 500;`).
- [ ] **Sem código morto.** Imports, props, funções não usadas — remover
      antes do PR. \"Step 0\" de CLAUDE.md.
- [ ] **Sem comentário redundante.** Comentário explica *por quê*, não
      *o quê*. Remover comentários que descrevem o nome da função.
- [ ] **Teste novo para cada caminho novo?** Positivo + pelo menos um
      negativo (\"não deveria disparar\"). Ver `l1-l2-invariants.md`.
- [ ] **Commit atômico.** Uma mensagem de commit = uma mudança lógica.
      Separar cleanup de feature.

## Gate 4 — Fidelidade à issue

Quando o commit referencia uma issue (`Closes #N`, `Refs #N`, ou branch
nomeada `issue-N-*`):

- [ ] **Ler a descrição atual da issue antes de codar.** A seção
      \"Validação (2026-04-18)\" (quando presente) costuma ter escopo
      ajustado vs. o título original — esse é o escopo efetivo.
- [ ] **Cobertura dos \"critérios de pronto\".** Cada bullet sob \"Critério
      de pronto\" ou \"Critério de sucesso\" da issue tem evidência no PR
      (teste, screenshot, linha de log). Se algum bullet ficou fora,
      justificar na descrição do PR e abrir issue de follow-up.
- [ ] **Nada além do escopo.** Refactor oportunista de código vizinho vai
      em commit separado ou PR separado. A issue é o contrato.
- [ ] **Issue realmente resolvida?** Se a \"Validação\" concluiu que a
      premissa já estava implementada (ex: #14 config merge), a issue
      fecha com PR de **teste de regressão**, não de implementação.
      Fechar sem código é aceitável quando documenta o estado.

## Comandos obrigatórios antes do PR

Nesta ordem, parar no primeiro que falhar:

```bash
cargo fmt --check
cargo clippy -- \
  -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::panic -W clippy::arithmetic_side_effects \
  -D warnings
cargo test
cargo audit         # se Cargo.toml mudou
# cargo deny check licenses   # quando #3 entregar
```

Se a mudança toca hot path (L1, L2, handlers do daemon):

```bash
cargo bench --bench compression_bench -- --baseline master
```

Regressão > 10% em qualquer layer = **bloqueio** até investigar.

## Como reportar no PR

No corpo do PR, incluir seção:

```markdown
## Implementation Gate

- Segurança: [lista curta de itens relevantes verificados]
- Memória / overhead: [lista curta]
- Boas práticas: [confirmo clippy + fmt + test passando]
- Fidelidade à issue #N: [bullets dos critérios de pronto cobertos]
```

Não é burocracia: é o que distingue um PR mergeável de um que vai
regenerar bug em 3 meses.

## Relacionadas

- `clippy-gate.md` — os lints obrigatórios já automatizados
- `l1-l2-invariants.md` — invariantes que não podem ser violados
- `l4-context-injection.md` — garantias arquiteturais de L4
- `~/.claude/rules/rust-security-audit.md` (global) — auditoria de segurança
