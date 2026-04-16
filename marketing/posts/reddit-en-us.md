---
platform: reddit
language: en-US
suggested_subs:
  - r/rust             # Show-and-tell flair, weekends get more traffic
  - r/LocalLLaMA       # the audience that cares most about context budgeting
  - r/ClaudeAI         # direct user base
  - r/opensource       # contributor pool
  - r/programming      # general
title_char_budget: 300
recommended_title: "I built a Rust proxy that compresses 60–90 % of shell output before it reaches Claude Code — it's early and I need contributors"
flair_suggestion: "Show-and-tell"
seo_keywords:
  - LLM context compression
  - Rust Claude Code proxy
  - PostToolUse hook
---

<!--
Reddit: paste the recommended_title into the title field and the body
below "## TL;DR" into the post. Drop this frontmatter.
-->

## TL;DR

Project: <https://github.com/VALRAW-ALL/ntk>

I built **NTK (Neural Token Killer)** — a Rust daemon that sits between Claude Code and the LLM, compressing shell output (`cargo test`, `docker logs`, stack traces, etc) before it turns into context. Measured on real fixtures: **92 %** savings on repetitive Docker logs, **56–83 %** on stack traces, **< 20 ms** overhead on the regex + tokenizer layers. MIT-licensed, runs offline, no paid API. Early-stage and I need contributors.

---

## The problem

Anyone using Claude Code / Cursor / OpenCode knows the pain: every `Bash` the model runs, the **entire** output comes back as context on the next turn. A `cargo test` with 200 passing tests burns 1500+ tokens of noise. A 10-minute `docker logs -f` blows the session's context window.

Existing alternatives — RTK and shell-side regex filters — work for simple cases but are:

- Synchronous (added latency to the command itself)
- Semantically blind (filters what the author already knew was noise, not what the model would consider noise)
- Too specific (one rule per command category)

## How NTK fixes it

Four-layer pipeline running **asynchronously** via a `PostToolUse` hook against a local daemon:

```
Bash output → hook → POST /compress on :8765
  ├── L1  Fast Filter      regex / ANSI / template dedup / stack-trace filter
  ├── L2  Tokenizer-Aware  cl100k_base / path shortening / hash normalisation
  ├── L3  Local Inference  Phi-3 Mini via Ollama | Candle | llama.cpp (optional, > 300 tokens)
  └── L4  Context Injection reads Claude Code transcript, prefixes user intent into L3 prompt
```

Stack: Rust + axum + tokio + tiktoken-rs + candle + sqlx. Single binary, builds on Windows/macOS/Linux.

## Honest numbers (measured, not claimed)

All from `bench/microbench.csv` across 15 deterministic fixtures in `bench/fixtures/`:

- `docker_logs_repetitive` → **92 %**
- `node_express_trace`     → 83 %
- `cargo_test_failures`    → 68 %
- `python_django_trace`    → 62 %
- `stack_trace_java`       → 60 %
- `go_panic_trace`         → 56 %
- `php_symfony_trace`      → 33 %

L1+L2 overhead is under 20 ms even on pathological input. L3 only engages when post-L1+L2 output is still over 300 tokens.

## Where I need help (this is the actual ask)

I'm maintaining this solo. The list of things I can't cover well alone:

1. **Fixtures for new languages** — Elixir/Phoenix, Scala/Akka, Swift/iOS, Flutter/Dart, Clojure, Erlang/OTP. L1 currently covers Java, Python/Django, Ruby/Rails, Node/Express, Go, PHP/Symfony, Rust, .NET, JS/TS browser (React), React Native, Kotlin/Android.
2. **Port the hook to other editors** — Cursor, Aider, Zed, Continue, Windsurf. Claude Code and OpenCode are supported today.
3. **GPU benchmarks I can't run** — the README numbers for AMD, Apple Silicon and Intel AMX are partly estimates. Real measurements close the gap.
4. **Site translations** — `docs/app.js` has EN + PT blocks. ES/FR/DE/JA is pure copy-and-translate.
5. **Break an invariant** — 8 property-based tests guard L1/L2 behaviour (`cargo test --test compression_invariants`). Finding an input that violates any of them is a very valuable bug report.

Repo ships with `CONTRIBUTING.md`, `HOW_TO_OPEN_AN_ISSUE.md`, and `.claude/skills/add-stack-trace-language.md` — pre-scoped playbooks so you're not guessing what to touch. Most tasks fit in a single PR under an hour.

## Links

- Code: <https://github.com/VALRAW-ALL/ntk>
- Contributing: <https://github.com/VALRAW-ALL/ntk/blob/master/CONTRIBUTING.md>
- How to open an issue: <https://github.com/VALRAW-ALL/ntk/blob/master/HOW_TO_OPEN_AN_ISSUE.md>
- Landing page: <https://ntk.valraw.com>

## Questions for the thread

1. Which command / output category destroys your context the most in Claude Code / Cursor / similar?
2. Have you tried any existing solution? Which, and why did you stop?
3. For people who know filtering pipelines — am I reinventing a wheel you've already seen?

Harsh feedback welcome. "This is bad because X" ranks higher in my book than a silent upvote.
