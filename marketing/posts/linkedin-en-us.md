---
platform: linkedin
language: en-US
ideal_char_count: 1300-2000
hashtags:
  - "#Rust"
  - "#AI"
  - "#LLM"
  - "#OpenSource"
  - "#DeveloperTools"
  - "#ClaudeCode"
  - "#OpenToContribute"
  - "#SoftwareEngineering"
  - "#MachineLearning"
hook_strategy: "question + concrete number in the first 3 lines (above the 'see more' fold)"
---

<!--
LinkedIn: paste the body below straight into the composer. Hashtags go
at the end, pre-formatted. No inline images; LinkedIn prefers one
preview image attached to the link (use assets/rtk.webp or a screenshot
of ntk.valraw.com).
-->

Does your LLM agent also run `cargo test` and then waste 1,500 tokens re-reading "test ok" 200 times?

I spent the last months on that problem and ended up building something. Sharing here because it's open for collaboration.

---

**The problem**

Agent editors — Claude Code, Cursor, OpenCode — execute shell commands in a loop. Every output comes back into the model's context: a 10,000-line `docker logs`, a `tsc` run with repeated warnings, a 300-line stack trace. That eats the context window, raises latency, and — if you're on a paid API — raises cost.

**What I built**

NTK (Neural Token Killer): a Rust daemon that sits between the editor and the LLM, intercepting outputs via a `PostToolUse` hook and compressing them before they become context.

Four-layer pipeline:
→ L1 regex (ANSI strip, template dedup, stack-trace filter across 11 languages)
→ L2 cl100k_base tokenizer (path shortening, hash normalisation)
→ L3 local inference with Phi-3 Mini (Ollama / Candle / llama.cpp — optional)
→ L4 context injection reading the user's current intent from the session transcript

**Measured numbers (not estimates):**
— 92 % savings on repetitive Docker logs
— 56–83 % on stack traces (Java, Python, Go, Node, PHP, C# and more)
— < 20 ms overhead on the first two layers
— 100 % open-source, MIT, runs offline

**Why I'm posting this here:**

I started this project alone. To evolve, it needs people I can't reach by myself — devs with logs from languages we don't cover yet (Elixir, Swift, Flutter, Scala), people with GPUs I don't own for real benchmarks, translators for the docs, hook ports for other editors.

I'm calling it an "open initiative" in the README on purpose: this isn't a product yet — it's a collaborative effort starting now.

**If you:**
→ Spend hours with LLM agents, drop a comment with the command that destroys your context the most
→ Want to contribute, the repo ships a CONTRIBUTING.md with pre-scoped tasks (most fit in one PR under an hour)
→ Just want to follow, a star keeps it on the radar

Link: github.com/VALRAW-ALL/ntk

And the question I keep turning over: **is semantic context compression the next implicit standard for LLM agents, or just a niche optimisation?** Would love to hear from people working the same problem.

---

#Rust #AI #LLM #OpenSource #DeveloperTools #ClaudeCode #OpenToContribute #SoftwareEngineering #MachineLearning
