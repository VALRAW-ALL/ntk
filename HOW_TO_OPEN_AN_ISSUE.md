# How to open an issue

NTK is maintained on a shoestring. Clear, well-scoped issues land fixes
in **days**; vague ones sit for weeks. This guide walks you through the
5-minute version of "how to file an issue that gets attention".

## Before you open one

1. **Search existing issues** — <https://github.com/VALRAW-ALL/ntk/issues?q=is%3Aissue>.
   Open or closed, there's a good chance someone already hit it.
2. **Update to the latest release** — run `ntk --version`. If you're not
   on the current `v0.2.x` from <https://github.com/VALRAW-ALL/ntk/releases>,
   upgrade first; the bug may already be fixed.
3. **Try to isolate** — can you reproduce with a minimal command output
   pasted to `ntk test-compress <file>`? If yes, attach that file.

## Which template do I pick?

On <https://github.com/VALRAW-ALL/ntk/issues/new/choose> you'll see three
templates plus two contact links. Pick in this order:

| Template | When |
|---|---|
| **🐛 Bug report** | Something is broken — wrong compression, crash, lost error line, daemon returns 500, hook exits non-zero. |
| **✨ Feature request** | "NTK should also dedup this category / detect this pattern / support this editor". The pipeline still works, but is missing something. |
| **🧱 New stack-trace language** | You want L1 to recognise framework frames from a language we don't cover (Elixir, Swift, Scala, Flutter, Erlang, …). |
| *Contact link* → **Contributing guide** | You want to send a PR — read [CONTRIBUTING.md](CONTRIBUTING.md) first, then open a PR directly. |
| *Contact link* → **Discussions** | Open-ended question ("does NTK work with X?"), sharing session savings, or asking for design feedback on a large change before opening a PR. |

Blank issues are disabled on purpose — the templates exist to save both
of our time.

## What goes in a good bug report

The `bug_report` template is intentionally short. These four fields are
**non-negotiable**:

1. **Steps to reproduce** — a concrete sequence, ideally including a
   command output we can paste into `ntk test-compress`.
2. **What you expected vs what happened** — one sentence each.
3. **Environment** — NTK version, OS, daemon backend (`ollama` /
   `candle` / `llama_cpp`), GPU if any. Copy-pasted output of
   `ntk status` covers all of this.
4. **Config** — `~/.ntk/config.json` (or project-local `.ntk.json`)
   with secrets redacted.

Everything else (logs, workarounds, screenshots) is welcome but
optional.

### Good bug report example

> **Title:** [bug] cargo test output loses the "---- stderr ----" block
>
> **What happened:** When compressing `cargo test` failure output,
> NTK keeps the "FAILED" summary but drops the stderr section that
> contains the actual panic message, so Claude can't see why the
> test failed.
>
> **Steps to reproduce:**
> 1. Save the attached `cargo_test_panic.txt` (included).
> 2. Run `ntk test-compress cargo_test_panic.txt`.
> 3. Observe that the panic message "thread 'tests::x' panicked at"
>    is not in the output.
>
> **Expected:** Panic line survives L1 (invariant #1 from
> `.claude/rules/l1-l2-invariants.md`).
>
> **Actual:** It is dropped; only the `test result: FAILED` line remains.
>
> **Environment:** ntk 0.2.29 / Ubuntu 24.04 / candle backend / CPU-only
> / gpu_layers=0.
>
> **Config:** defaults; no `.ntk.json` in project.

Notice what the example does **not** include: no paragraphs of
background, no apologies, no speculation about the cause. Just enough
for a maintainer to reproduce in under 60 seconds.

## What goes in a good feature request

The `feature_request` template has a checklist — tick the pipeline
layer(s) your proposal touches:

- `[ ]` This can be done by adding a fixture in `bench/fixtures/`
- `[ ]` This needs an L1 regex / framework pattern
- `[ ]` This needs L2 tokenizer work
- `[ ]` This needs an L3 prompt change
- `[ ]` This needs an L4 context-format change
- `[ ]` This needs an editor hook port
- `[ ]` Docs / i18n

Ticking even one box saves the maintainer 10 minutes of figuring out
where the change lives. If nothing fits, explain in your own words in
the "Proposed solution" section.

## What we generally *cannot* help with

- **"Make NTK faster for my specific model"** without a reproducible
  benchmark. Micro-optimisations without numbers are closed.
- **Private / licensed log samples.** If the bug only reproduces against
  a corporate log you can't share, redact it into a synthetic fixture
  that still triggers the bug.
- **Requests to remove telemetry** — it's already opt-out (see README).
- **Support for Windows XP / macOS 10.x / Ubuntu 18.04** — we target
  current LTS releases.

## After you open the issue

- Expect a first response in **2–5 days** (this is a solo project).
- If the issue needs a reproducer you haven't attached, we'll ask once
  and close as "needs info" after ~14 days of silence.
- If you want to fix it yourself, comment "I'll take this" and we'll
  assign. Most issues are labeled with the layer + difficulty, so you
  know roughly what you're signing up for.

## Shortcuts

- [Open a bug report](https://github.com/VALRAW-ALL/ntk/issues/new?template=bug_report.md)
- [Open a feature request](https://github.com/VALRAW-ALL/ntk/issues/new?template=feature_request.md)
- [Request a new stack-trace language](https://github.com/VALRAW-ALL/ntk/issues/new?template=stack_trace_language.md)
- [See all open issues](https://github.com/VALRAW-ALL/ntk/issues)

Thanks for taking the time. Good issues are what keep NTK improving.
