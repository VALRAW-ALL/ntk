# Layer 4 Prompt-Format Ablation

This document records the empirical basis for picking the default
`PromptFormat` enum variant in `src/compressor/layer4_context.rs`. The
bench infrastructure (`bench/prompt_formats.ps1`, issue #5) is in place;
what's missing is the actual run-and-record pass in hardware we trust.
Anyone with a GPU and an hour can contribute a measurement — open a PR
with updated numbers.

## Hypothesis being tested

Does injecting the user's recent intent into the L3 prompt measurably
improve compression ratio and/or error-signal preservation, and which
of the four format variants performs best?

Four formats compete:

| Variant | Shape (approx.) | Overhead in L3 prompt |
|---|---|---|
| `Prefix` (current default) | `"CONTEXT: User asked: <intent> — focus on relevance. <output>"` | ~30 tokens |
| `XmlWrap` | `"<intent>...</intent>\n<output>"` | ~15 tokens |
| `Goal` | `"User goal: <intent> — extract only info that advances this goal.\n<output>"` | ~40 tokens |
| `Json` | `'{"intent":"<intent>","output":"<output>"}'` | JSON escaping overhead |

## Methodology

```powershell
# Warm the llama.cpp server / Ollama first so load times don't pollute
# the first-run latency.
ntk stop; ntk start
# Wait until `ntk status` reports 'llama-server ready'.

# Full A/B: every (format, fixture, context_enabled) combination, 3 reps.
pwsh bench/prompt_formats.ps1 -CompareContext $true
```

Outputs:

- `bench/prompt_formats.csv` — per-combination (format, fixture, context_enabled, tokens_before, tokens_after, ratio, layer, latency_ms)
- Summary tables printed to stdout: ratio by format, ratio by (format, context), per-fixture delta (with - without context)

## Decision criterion

1. **If any format beats `Prefix` by > +2 pp average ratio** across the
   L3-triggering fixtures AND preserves error signal (every `error:`,
   `ERROR`, `FAILED`, `panic:` line from the input survives in the
   output) — flip `#[default]` to that variant. Commit with the CSV.

2. **If no format beats `Prefix` by > +2 pp** — document the null
   result and leave the default alone.

3. **If context on/off delta is ≤ +2 pp for ALL formats** — L4 is not
   paying its latency cost. Flip `compression.context_aware` to
   `false` by default, keeping the opt-in path for users who notice a
   qualitative improvement.

## Results (unfilled)

**Hardware:** *(fill in: CPU model, GPU model if any, OS)*
**llama.cpp build:** *(e.g. `b8840-9e5647aff`)*
**Model:** Phi-3 Mini Q5_K_M
**Run date:** *(YYYY-MM-DD)*

### Ratio by format (averaged across L3-triggering fixtures)

| Format | Avg ratio | Δ vs Prefix |
|---|---:|---:|
| Prefix  | — | — |
| XmlWrap | — | — |
| Goal    | — | — |
| Json    | — | — |

### Context impact per format

| Format | Ratio w/ context | Ratio w/o context | Δ |
|---|---:|---:|---:|
| Prefix  | — | — | — |
| XmlWrap | — | — | — |
| Goal    | — | — | — |
| Json    | — | — | — |

### Error-signal preservation

*(count of input lines matching `error:|ERROR|FAILED|panic:|Exception|Traceback` that survived vs total)*

| Format | Preserved | Dropped |
|---|---:|---:|
| Prefix  | — | — |
| XmlWrap | — | — |
| Goal    | — | — |
| Json    | — | — |

### Conclusion

*(fill in after running)*

### Reproducibility

- CSV attached under `bench/prompt_formats.csv` (not committed — too
  hardware-dependent; attach to PR description as a code block instead)
- Submit the equivalent JSON via `ntk bench --submit --l3` (#15) so
  hardware context + per-fixture latencies are machine-readable
