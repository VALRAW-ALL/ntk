---
name: add-stack-trace-language
description: Playbook for extending L1's stack-trace filter to a new language or framework. Covers classifier patch, fixture creation, tests, and docs updates.
---

# Skill: Add Stack-Trace Language Support

Use this skill when the user asks to "add support for stack traces from
X" (e.g. Erlang, Haskell, Scala, Swift, Flutter). The skill is a strict
checklist — skipping a step leaves the feature half-wired.

## Step 0 — Gather a real trace

Don't invent one. Ask the user to paste an actual stack trace from the
target language, or generate one yourself with a minimal crashing
program. Ideal trace length is 15–30 lines with **at least** one user
frame and a clear error header (Exception/panic/TypeError/etc).

## Step 1 — Identify the framework signature

Read the trace and identify markers that distinguish **framework / runtime
frames** from **user frames**:

- Does every framework frame start with `at <ecosystem>.<package>.`? → use `starts_with`
- Do framework frames always sit under a known path (`/vendor/`, `node_modules/X/`)? → use `contains`
- Is the user entrypoint ambiguous with framework (e.g. `main.main` vs `runtime.main`)? → add an exclusion with `return false` **before** the framework check

## Step 2 — Patch `is_framework_frame`

Add a new `--- <Language> / <Framework> ---` block inside
`src/compressor/layer1_filter.rs`, following the existing conventions
(see `.claude/rules/stack-trace-classifier.md`).

## Step 3 — Handle multi-line frame units (if applicable)

If the language emits `header\nbody` pairs where the body alone looks
like user code, extend `is_framework_frame_unit` with the pairing rule.

## Step 4 — Add an in-module unit test

File: `src/compressor/layer1_filter.rs` inside `#[cfg(test)] mod tests`.

```rust
#[test]
fn test_stack_trace_<lang>_filter() {
    let input = "<real 15-line trace>";
    let result = filter(input);
    assert!(result.output.contains("<error header>"));      // invariant #1
    assert!(result.output.contains("<user frame fn name>")); // invariant #3
}
```

## Step 5 — Add a bench fixture

Two files in `bench/fixtures/`:

- `<lang>_<framework>_trace.txt` — the raw trace
- `<lang>_<framework>_trace.meta.json`:
  ```json
  {
    "category":       "stack_trace",
    "description":    "<ecosystem> <error> with heavy <markers> framework frames.",
    "min_ratio":      0.40,
    "expected_layer": 2,
    "command":        "<what produced it, e.g. 'dotnet run'>"
  }
  ```

Choose `min_ratio` from the table in `stack-trace-classifier.md`.

## Step 6 — Mirror the fixture in the PowerShell generator

`bench/generate_fixtures.ps1` — add a new `Write-Fixture '<name>' $body @{…}`
block at the end. The generator must produce the **exact** same content
as the hand-written `.txt` so a fresh `bench/fixtures` dir reproduces
the committed library.

## Step 7 — Update the metrics table

Two files:

- `docs/index.html` — new `<tr>` row. If you haven't measured compression
  yet, use:
  ```html
  <span class="range-badge" style="color:var(--color-texto-mutado)">N/A</span>
  ```
  Never commit an invented percentage.
- `docs/app.js` — EN + PT translation keys:
  ```js
  'met.fx_<short>': '<English name>',    // en block
  'met.fx_<short>': '<Portuguese name>', // pt block
  ```

## Step 8 — Verify

Run the clippy gate + tests (CI flags, no `--all-targets`):

```bash
cargo fmt --check
cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used \
                -W clippy::panic -W clippy::arithmetic_side_effects \
                -D warnings
cargo test --lib compressor::layer1_filter
cargo test --test compression_invariants
```

If the proptest fails, fix the classifier — **do not** relax the
property. The property is the contract.

## Definition of done

- [ ] Patch in `is_framework_frame` (+ optional pairing in `_unit`)
- [ ] In-module test added and passing
- [ ] Fixture `.txt` + `.meta.json` committed
- [ ] `generate_fixtures.ps1` reproduces the fixture identically
- [ ] `docs/index.html` row + EN/PT translation keys
- [ ] clippy gate + full test suite green
- [ ] List of currently-supported languages in
      `.claude/rules/stack-trace-classifier.md` updated
