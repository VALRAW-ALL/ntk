---
platform: reddit
language: pt-BR
suggested_subs:
  - r/brdev
  - r/programacao
  - r/devbrasil
title_char_budget: 300
recommended_title: "[Show-and-tell] Criei um proxy em Rust que comprime 60–90 % da saída de comandos antes de virar contexto no Claude Code — e preciso de ajuda para evoluir"
flair_suggestion: "Show-and-tell / Open Source"
seo_keywords:
  - compressor de tokens LLM
  - proxy rust claude code
  - hook PostToolUse
---

<!--
Reddit: cole o título e o corpo abaixo. Remova este frontmatter ao publicar.
-->

## TL;DR

Projeto: <https://github.com/VALRAW-ALL/ntk>

Fiz o **NTK (Neural Token Killer)** — um daemon em Rust que fica entre o Claude Code e o LLM e comprime a saída de comandos (`cargo test`, `docker logs`, stack traces, etc) antes que ela vire contexto. Medido em fixtures reais: **92 %** de economia em Docker logs repetitivos, **56–83 %** em stack traces, overhead de **< 20 ms** nas camadas regex + tokenizer. MIT, roda offline, sem API paga. É early-stage e preciso de contribuidores.

---

## Qual o problema

Quem usa Claude Code / Cursor / OpenCode sabe: todo `Bash` que o modelo roda, a saída **inteira** volta pro contexto na próxima turn. Um `cargo test` com 200 testes passando come 1500+ tokens de ruído. Um `docker logs -f` de 10 min manda sua sessão direto pro teto do context window.

A solução mais comum hoje — RTK e filtros por regex no shell — funciona pra caso simples mas é:

- Síncrona (adiciona latência na cadeia do comando em si)
- Cega semanticamente (filtra o que o autor da regra conhece, não o que o modelo ia considerar ruído)
- Específica demais (uma regra por categoria de comando)

## Como o NTK resolve

Uma pipeline em 4 camadas, rodando no hook `PostToolUse` **assíncrono** ao comando, em um daemon local:

```
Saída do Bash → hook → POST /compress no :8765
  ├── L1  Fast Filter      regex / ANSI / dedup por template / stack-trace filter
  ├── L2  Tokenizer-Aware  cl100k_base / encurtamento de paths / normalização de hashes
  ├── L3  Local Inference  Phi-3 Mini via Ollama | Candle | llama.cpp (opcional, > 300 tokens)
  └── L4  Context Injection lê o transcript do Claude Code e prefixa sua intenção no prompt da L3
```

Stack: Rust + axum + tokio + tiktoken-rs + candle + sqlx. Binário único, compila em Windows/macOS/Linux.

## Números honestos (medidos, não inventados)

Todos vêm do `bench/microbench.csv` rodando contra 15 fixtures em `bench/fixtures/`:

- `docker_logs_repetitive` → **92 %**
- `node_express_trace`     → 83 %
- `cargo_test_failures`    → 68 %
- `python_django_trace`    → 62 %
- `stack_trace_java`       → 60 %
- `go_panic_trace`         → 56 %
- `php_symfony_trace`      → 33 %

Overhead L1+L2: < 20 ms no pior caso. L3 só acorda se pós-L1+L2 ainda tem > 300 tokens.

## O que PRECISA de ajuda (é aqui que você entra)

Tô sozinho no projeto. A lista de coisas que não consigo cobrir bem:

1. **Fixtures de linguagens novas** — Elixir/Phoenix, Scala/Akka, Swift/iOS, Flutter/Dart, Clojure, Erlang/OTP. Hoje o filtro L1 cobre Java, Python/Django, Ruby/Rails, Node/Express, Go, PHP/Symfony, Rust, .NET, JS/TS browser (React), React Native, Kotlin/Android.
2. **Portar o hook pra outros editores** — Cursor, Aider, Zed, Continue, Windsurf. Claude Code e OpenCode já rodam.
3. **Benchmarks em GPU que não tenho** — os números de AMD, Apple Silicon e Intel AMX no README são parcialmente estimados.
4. **Tradução do site** — `docs/app.js` tem EN + PT. ES/FR/DE/JA é só copiar e traduzir.
5. **Quebrar invariantes** — temos 8 property-based tests (`cargo test --test compression_invariants`). Achar uma entrada que viole qualquer um é ouro.

Repo já vem com `CONTRIBUTING.md` + `HOW_TO_OPEN_AN_ISSUE.md` + `.claude/skills/add-stack-trace-language.md` com playbooks pré-definidos. A maior parte das tarefas cabe em um PR único em menos de uma hora.

## Links

- Código: <https://github.com/VALRAW-ALL/ntk>
- Como contribuir: <https://github.com/VALRAW-ALL/ntk/blob/master/CONTRIBUTING.md>
- Como abrir uma issue: <https://github.com/VALRAW-ALL/ntk/blob/master/HOW_TO_OPEN_AN_ISSUE.md>
- Landing: <https://ntk.valraw.com>

## Perguntas pra conversa

1. Qual comando (ou categoria de output) mais detona seu contexto no Claude Code / Cursor?
2. Já tentou alguma solução pra isso? Qual e por que parou?
3. Pra quem conhece de filtros e pipelines — estou reinventando roda em algum ponto óbvio?

Aceito críticas duras. "Isso é ruim porque X" vale mais do que upvote silencioso.
