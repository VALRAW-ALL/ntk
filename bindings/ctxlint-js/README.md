# @ctxlint/core — JS/TS reference impl

Second-runtime binding for RFC-0001 ([`docs/rfcs/0001-context-linter-spec.md`](../../docs/rfcs/0001-context-linter-spec.md)).
Tracks issue #26.

The Rust impl in `src/compressor/spec_loader.rs` is the primary
reference; this package exists to prove the rule-file format is
runtime-independent. When the two drift on the same input + rule
file, the spec is wrong — not either implementation.

## Status

**Proof of concept.** 9 parity tests mirror the 10 unit tests on
the Rust side (the tenth is a YAML schema rejection that's
language-intrinsic and doesn't round-trip). Not published to npm
yet — that happens when RFC-0001 reaches `v0.1-accepted` per
§18 of the RFC.

## Usage

```js
import { loadRuleFile, applyRuleFile } from '@ctxlint/core';

const rules = loadRuleFile('./rules/stack_trace/python.yaml');
const result = applyRuleFile(pythonTraceText, rules);

console.log(result.output);
//   Compressed text — framework frames collapsed, error signal
//   preserved, first + last user frames kept.
console.log(result.applied);
//   ['py.stack.site_packages', 'py.stack.asgi_runtime']
console.log(result.invariantRejected);
//   Rule IDs that the runtime refused to apply because their
//   transform would have dropped error signal.
```

## Primitives implemented

All four from RFC-0001 §5:

- `frame-run` + `collapse-run` — framework-frame collapse across
  any language (Java, Python, Go, Node, Ruby, PHP, Rust, .NET, etc)
- `line-match` + `delete` | `rewrite` — per-line regex/prefix
  filter with optional captures on rewrite
- `template-dedup` + `dedup` — normalize volatile fields then
  group identical templates (skips blanks — regression-guarded)
- `prefix-factor` + `factor-prefix` — extract longest common prefix

## Classifier kinds

`contains` · `starts_with` · `equals` · `regex`

## Invariants enforced

Currently one, matching the Rust impl:

- `preserve_errors` — regex-scan of error/panic/Traceback/FAILED
  markers before and after; rule is rejected post-hoc if the
  transform would have reduced that count.

`preserve_first_frame` / `preserve_last_frame` are structurally
enforced by the `collapse-run` implementation (always emits the
first and last frame of a collapsed run verbatim).

## Running tests

```bash
npm install
node --test test.js
```

## Known gaps vs Rust reference

- `spec_version` comparison uses `String(v)` cast because the `yaml`
  package parses `0.1` as a number; Rust's `serde_yaml` deserializes
  into `String`. Functionally equivalent but a visible diff.
- No feature-flag / config plumbing — just the loader + primitives.
  A Continue / VSCode integration wraps this package into an MCP
  server or tool hook (see `docs/editor-integrations.md`).

## License

MIT. Same as NTK itself.
