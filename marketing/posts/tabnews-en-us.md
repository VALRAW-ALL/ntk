---
platform: tabnews
language: en-US
channel: pub
suggested_tags: [rust, llm, opensource, claude, ai, developer-tools, cli]
seo_keywords:
  - LLM context compression
  - Claude Code compression proxy
  - NTK neural token killer
  - rust CLI token saver
  - PostToolUse hook Claude Code
title_char_budget: 150
recommended_title: "NTK: a Rust daemon that compresses 60–90 % of shell output before it reaches Claude Code"
---

<!--
This is the body only. Paste the recommended_title into TabNews's title
field and the content below "## TL;DR" into the editor. Do not include
this frontmatter when publishing.
-->

## TL;DR

I built **NTK (Neural Token Killer)**, a local Rust proxy that sits between Claude Code (or any editor with a PostToolUse-style hook) and the LLM. It intercepts `Bash` / `cargo test` / `docker logs` / `tsc` output and compresses it before it lands in the model's context. Measured numbers: up to **92 %** on repetitive Docker logs, **56–83 %** on stack traces across languages, **< 20 ms** overhead on the regex + tokenizer layers.

The project is **MIT-licensed** and very early. I need help testing, adding fixtures for languages we don't cover yet, translating the docs, porting the hook to other editors, and benchmarking on GPUs I don't own.

- Repo: <https://github.com/VALRAW-ALL/ntk>
- Contributing: <https://github.com/VALRAW-ALL/ntk/blob/master/CONTRIBUTING.md>
- How to open an issue: <https://github.com/VALRAW-ALL/ntk/blob/master/HOW_TO_OPEN_AN_ISSUE.md>

---

## The problem

If you use Claude Code, Cursor, OpenCode or any other LLM-agent editor, every time the model runs a `Bash` command **the entire output** is shipped back into the model's context on the next turn. A `cargo test` with 200 passing tests, a 10 000-line `docker logs`, a `tsc` run with repeated warnings — all that burns thousands of tokens that, from the model's perspective, are noise.

Practical consequences:

1. **Context window caps faster** → the session hits the wall mid-task.
2. **Higher response latency** → the model processes more input tokens.
3. **Higher cost** → if you're on a paid API.
4. **The model loses focus** → verbose output in context dilutes the signal (errors, diffs, warnings).

Existing solutions — RTK, shell-side regex filters — work for simple cases but are **synchronous** and **semantically blind**: they drop what the rule author knew was noise, not what the *model* would treat as noise.

## The idea behind NTK

A 4-layer pipeline that runs asynchronously via a `PostToolUse` hook:

```
Bash output
  → PostToolUse hook
    → HTTP POST /compress on local daemon
      → L1 Fast Filter   (regex, < 1 ms): ANSI, template dedup, stack-trace filter
      → L2 Tokenizer     (cl100k_base, < 5 ms): path shortening, BPE, normalisation
      → L3 Local Inference (Phi-3 Mini via Ollama/Candle/llama.cpp, only when > 300 tokens)
      → L4 Context Injection: prepends the user's current intent to the L3 prompt
  → Model context
```

L1 and L2 are always on and add negligible latency. L3 only fires when post-L1+L2 output is still over 300 tokens — that avoids the 300-800 ms inference tax on small outputs like `git status`. L4 reads the Claude Code session transcript to know the user's current question and prepends it to the compression prompt, which measurably improves ratios when the neural layer engages.

## Real numbers

All figures below come from `bench/microbench.csv` produced by `bench/run_all.ps1` against 15 deterministic fixtures in `bench/fixtures/`. No estimates.

| Fixture | Scenario | L1+L2 savings |
|---|---|---:|
| `docker_logs_repetitive`   | Logs with repeated timestamps | **92 %** |
| `node_express_trace`       | Node.js stack trace with node_modules/express | 83 % |
| `cargo_test_failures`      | `cargo test` with 1 failure out of 50 | 68 % |
| `python_django_trace`      | Django + gunicorn/asgiref stack | 62 % |
| `stack_trace_java`         | Spring/Tomcat/CGLIB | 60 % |
| `go_panic_trace`           | Go panic + goroutine dumps | 56 % |
| `php_symfony_trace`        | Symfony/HttpKernel + /vendor | 33 % |

L3 (neural inference) pushes these further for unstructured output, but Phi-3 Mini's CPU runtime is ~60 s per call without a GPU, so in practice you only enable it with CUDA/Metal acceleration.

## Why Rust

- Deterministic latency — every `Bash` call can be intercepted; overhead needs to be predictable.
- Single binary, zero runtime deps — if your shell starts, NTK starts.
- Cross-platform without `#ifdef` hell — Windows, macOS, Linux build from the same source.
- Strong typing + backend enum — swapping Ollama for Candle or llama.cpp is one `match` arm.

## Where I need help (this is where you come in)

I started the project solo. The list of things I **can't do well alone** is longer than what I can:

1. **Fixtures for new languages** — each fixture is a `.txt` + `.meta.json` pair. Still open: Elixir/Phoenix, Scala/Akka, Swift/iOS, Flutter/Dart, Clojure, Erlang/OTP. Any real log is a starting point.
2. **Port the hook to other editors** — Claude Code and OpenCode work today. Cursor, Aider, Zed, Continue, Windsurf all have similar JSON schemas. One editor = one self-contained shell script.
3. **Benchmarks on hardware I don't have** — the README numbers for AMD (Vulkan), Apple Silicon (Metal), and Intel AMX are partly estimates. If you own one, running `ntk model bench` and sending a CSV closes the gap.
4. **Translations** — the site (`docs/`) has EN + PT blocks. ES/FR/DE/JA is pure copy-and-translate, no code.
5. **Break an invariant** — `cargo test --test compression_invariants` holds 8 invariants (errors survive, idempotency, etc). An input that violates any of them is one of the most valuable bug reports possible.

`CONTRIBUTING.md` and `HOW_TO_OPEN_AN_ISSUE.md` in the repo list pre-scoped tasks. Most land as a single PR in under an hour.

## Honest trade-offs

- **NTK is not a remote-LLM replacement** — L3 uses Phi-3 Mini (3.8B). It's good at summarising structured output; it doesn't write your code.
- **Privacy** — nothing leaves your machine; telemetry is opt-out and doesn't send files/paths/contents.
- **Combined NTK+RTK not yet measured** — an open task. Today the "NTK+RTK combined" panel on the site reads `N/A` because I don't have a real number.

## Open questions (I'd love your take)

1. Do you use Claude Code / any LLM agent that runs local commands? Which command wrecks your context the most?
2. If this tool had existed before and you knew about it, would you have installed it? What would make you **not** install it?
3. Any language / framework you'd want supported first?

Any feedback (including "this is a bad idea because X") is worth more than a silent star.

Thanks for reading. If you got this far and want to contribute, the shortest path is <https://github.com/VALRAW-ALL/ntk/blob/master/CONTRIBUTING.md>.
