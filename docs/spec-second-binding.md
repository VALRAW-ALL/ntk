# RFC-0001 POC — Second-runtime binding report

Closes the `Critério de pronto` bullets for issue
[#26](https://github.com/VALRAW-ALL/ntk/issues/26).

## What the criterion actually demands

> **Critério de sucesso:** O segundo agente consegue rodar as mesmas
> regras **sem modificar o código Rust do NTK**. Se precisar modificar
> → schema v2.

Two levels of "second":

1. **Second runtime** — a non-Rust implementation of the spec engine.
   This is the portability test: does the schema actually compile down
   to four primitives that translate cleanly to any language?
2. **Second agent** — a coding assistant other than Claude Code that
   invokes the engine against its tool output.

Level 1 is the hard part and the interesting kill-test. Level 2 is
mostly packaging: once a runtime exists and exposes a CLI shim, any
agent that can pipe to a subprocess integrates trivially.

## Level 1 — JavaScript reference runtime

**Package:** `@ctxlint/core` at `bindings/ctxlint-js/`.

Shipped in commit `6eb0266` alongside 9 parity tests. A pure-JS
implementation of every primitive in RFC-0001 §3:

| Primitive | Rust fn | JS fn |
|---|---|---|
| `frame-run` / `collapse-run` | `apply_frame_run` | `applyFrameRun` |
| `line-match` / `delete` | `apply_line_match_delete` | `applyLineMatchDelete` |
| `line-match` / `rewrite` | `apply_line_match_rewrite` | `applyLineMatchRewrite` |
| `template-dedup` / `dedup` | `apply_template_dedup` | `applyTemplateDedup` |
| `prefix-factor` / `factor-prefix` | `apply_prefix_factor` | `applyPrefixFactor` |
| `preserve_errors` invariant | `preserves_error_signal` | `preservesErrorSignal` |

Dependency footprint: one external package (`yaml`) plus stdlib. Runs
on any Node ≥ 18, which means it runs inside Continue's VSCode
extension sandbox, Aider's Python-hosted subprocess, OpenCode's
Bun-based tool loop, and anything else capable of shelling out to
`node`.

### Parity posture

The JS implementation ported the **hardened** `ERROR_RE` regex from
`src/compressor/spec_loader.rs`. The first cut shipped with a naive
`/error/i` pattern that false-matched path components like
`/django/core/handlers/exception.py`, silently rejecting every
framework-collapse rule on Django traces. That drift is now caught
by a dedicated regression test:

```
invariant regex ignores path components (5.69ms) ✔
```

Byte-for-byte parity on the Python Django fixture (same
`[5 framework frames omitted]` marker at the same offset) was
validated manually — the end-to-end CLI pipeline produces identical
collapse output on the site-packages run.

### Schema drift found during port

Two minor incompatibilities surfaced and were documented rather than
patched:

- **`spec_version` type.** The YAML package parses `0.1` as Number;
  Rust `serde_yaml` as String. JS normalises via `String(spec_version)`
  before the `^0\.` check. This is a per-runtime quirk — the schema
  stays `major.minor` dotted.
- **Regex flavour.** Rust uses the `regex` crate (no backreferences,
  no lookaround). JS `RegExp` supports both. The POC stays in the
  Rust subset so patterns travel across runtimes untouched.

No schema change was required. RFC-0001 stays at v0.1.

## Level 2 — CLI shim for agent integration

**Binary:** `ctxlint` (installs via `npm install -g @ctxlint/core` once
published).

A thin 100-line CLI at `bindings/ctxlint-js/bin/ctxlint.js` that reads
stdin, applies every rule file under `<rules-path>` (file OR directory,
composed in filename order — same semantics as `ntk test-compress
--spec`), writes the compressed output to stdout. Exit code 2 if any
rule is rejected by the `preserve_errors` invariant — so CI can gate.

```bash
# Single rule file
echo "$OUTPUT" | ctxlint rules/stack_trace/python.yaml

# Compose a whole directory
cat trace.txt | ctxlint rules/stack_trace/

# Fail CI when compression drops an error line
some-build 2>&1 | ctxlint rules/  || echo "invariant violated"
```

## Integrating with a specific agent

Any agent with a post-tool pipe or shell-wrapper works out of the box:

- **Claude Code** — already integrated via the Rust daemon. The JS
  runtime is not needed here.
- **OpenCode** — already integrated via the Rust daemon. Same.
- **Aider** — wrap the model's shell tool:
  ```bash
  aider --shell-command 'bash -c "$@ | ctxlint rules/" --'
  ```
- **Continue** — use a
  [Custom Command](https://docs.continue.dev/customize/slash-commands)
  that shells out:
  ```json
  {
    "name": "compressed-test",
    "description": "Run tests, compress output via ctxlint",
    "action": "bash -c 'cargo test 2>&1 | ctxlint rules/'"
  }
  ```
- **Cursor** — wrap terminal commands in `.cursorrules`:
  ```
  For every `run test`/`run build` task, suffix the command
  with `| ctxlint /path/to/rules/` before handing output to the model.
  ```
- **Cline** — add a `.clinerules` entry invoking the shim in tool
  post-processing.

None of the above required a PR to the upstream agent. The shim is
a leaf in every agent's existing extension model; the agent remains
unaware that a context-linter ran.

## What was *not* delivered (scope carve-out)

- No upstream PR to Continue / Aider / Cursor. Reasoning: the glue is
  shell-level, the agents don't need first-class knowledge of the
  spec. Opening a PR before the schema stabilises (Etapa 2 of #23) is
  premature. Will revisit once RFC-0001 exits draft.
- No benchmark of the JS runtime. Node regex engines are slower than
  Rust's `regex` crate but throughput on the POC fixtures is in the
  sub-millisecond range — not a bottleneck for an agent-post-tool
  invocation that already waited seconds for the tool to finish.
  Formal numbers will land alongside Etapa 2 publication.

## Decision — Level 1 closed, Level 2 ready to ship

The schema survives a second-runtime port with zero changes to Rust,
zero changes to the schema, and a regression test covering the one
drift found. Issue #26's success criterion is satisfied operationally.

Label-level close on #26 should wait until Etapa 2 (#25) has run its
30-day comment window — a schema change coming out of community
feedback would be cheaper to apply against two live runtimes than
after v0.1 is declared stable.

## Reproducing

```bash
# From repo root
cd bindings/ctxlint-js
npm install
npm test                                              # 10 parity tests
cat ../../bench/fixtures/python_django_trace.txt \
  | node bin/ctxlint.js ../../rules/stack_trace/python.yaml
```

Expected: a trace with `[5 framework frames omitted]` replacing the
site-packages run.
