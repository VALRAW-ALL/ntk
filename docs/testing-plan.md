# NTK — Token Reduction Test Plan

> Planning document for measuring NTK's real impact on Claude Code token
> consumption. Not an implementation spec — each section ends with open
> questions that need to be closed before coding.

---

## Goal

Quantify, with reproducible numbers, how much NTK actually reduces the
tokens billed by the Anthropic API when Claude Code runs Bash tools.
Produce evidence for:

1. **Per-layer contribution** — how many tokens does L1 remove? L2? L3?
2. **End-to-end savings** — run the same prompt with and without the
   NTK hook; measure the delta in `usage.input_tokens` recorded by
   Claude Code itself.
3. **Where NTK is worth it** — which tool outputs get the biggest
   reduction? Are there categories where NTK adds latency without
   saving enough tokens to be worth it?

Non-goals (for this plan):

- Measuring inference quality / semantic fidelity of L3 output.
- Benchmarking llama.cpp vs Ollama latency.
- Evaluating UX of the wizard / installer.

---

## Data we already have

| Signal | Source | Notes |
|---|---|---|
| Per-compression token counts | `POST /compress` response: `tokens_before`, `tokens_after`, `layer`, `ratio` | Only the **final** layer's numbers; L1/L2 intermediate totals are lost. |
| Aggregate session metrics | `GET /metrics` — `{total_original_tokens, total_compressed_tokens, layer_counts, average_ratio}` | Per-session, in-memory, reset on daemon restart. |
| Historical compressions | `~/.ntk/metrics.db` (SQLite) | Rows persisted per compression; schema in `src/metrics.rs`. |
| Claude Code turn usage | `~/.claude/projects/<project>/<session-id>.jsonl` | Each `assistant` event has `message.usage.{input_tokens, cache_read_input_tokens, cache_creation_input_tokens, output_tokens}`. |

## Data we do NOT have yet

| Missing signal | Why needed | How to get it |
|---|---|---|
| Raw input received by daemon | Can't replay / audit compressions | Add persistence to `POST /compress` handler. |
| Output after L1 (before L2 runs) | Can't attribute savings to L1 alone | Expose in response + save snapshot. |
| Output after L2 (before L3 runs) | Same as above for L2 | Same. |
| Final compressed output (persisted) | Can't reproduce what went to Claude | Save alongside the raw input. |

---

## Architecture of the test harness

```
┌─ microbench (fixture replay) ──┐     ┌─ macrobench (real session) ─────┐
│ bench/fixtures/*.txt           │     │ bench/prompts/baseline.md       │
│   ↓                            │     │   ↓                             │
│ bench/replay.sh                │     │ Run A: hook uninstalled         │
│   foreach fixture:             │     │   → transcripts/A.jsonl         │
│     POST /compress             │     │ Run B: hook active              │
│     write input/output/stats   │     │   → transcripts/B.jsonl         │
│       to ~/.ntk/logs/...       │     │   → ~/.ntk/logs/B/*.json        │
│   produce microbench.csv       │     │ bench/parse_transcript.py       │
└────────────────────────────────┘     │   produce macrobench.csv        │
                                       └─────────────────────────────────┘
                                                     ↓
                               bench/report.py → report.md
                                 - per-layer savings histogram
                                 - per-fixture ratio table
                                 - A-vs-B delta (tokens + estimated $)
                                 - overhead table (latency per layer)
```

---

## Implementation steps (ordered, each independently shippable)

### Step 1 — Expose per-layer metrics in `/compress`

**File:** `src/server.rs` (`CompressResponse`), `src/compressor/mod.rs`

Extend the response:

```json
{
  "compressed":       "<final output>",
  "layer":            3,
  "tokens_before":    3200,
  "tokens_after_l1":  1800,
  "tokens_after_l2":  1200,
  "tokens_after_l3":   280,
  "tokens_after":      280,
  "ratio":            0.91,
  "latency_ms": { "l1": 4, "l2": 12, "l3": 820 }
}
```

`tokens_after_l2` equals `tokens_after` when L3 does not trigger (below
threshold or fallback). `tokens_after_l3` is `null` when L3 is skipped.

### Step 2 — Persist compressions to disk (opt-in)

**New config field** in `ModelConfig`:

```json
"logging": {
  "save_compressions": false,
  "log_dir": "~/.ntk/logs"
}
```

Or gate via env var `NTK_LOG_COMPRESSIONS=1` (simpler, doesn't need
config migration).

When enabled, `POST /compress` writes one file per call:

```
~/.ntk/logs/2026-04-15/<uuid>.json
{
  "ts":           "2026-04-15T03:42:18.123Z",
  "command":      "cargo test",
  "cwd":          "/home/user/project",
  "input":        "<raw stdin from hook>",
  "after_l1":     "<L1 output>",
  "after_l2":     "<L2 output>",
  "after_l3":     "<L3 output or null>",
  "final":        "<what was returned>",
  "tokens":       { "before": 3200, "l1": 1800, "l2": 1200, "l3": 280 },
  "latency_ms":   { "l1": 4, "l2": 12, "l3": 820 },
  "layer_used":   3
}
```

Guardrails:
- Truncate `input` to `max_input_chars` (already enforced).
- Rotate / delete files older than 30 days (daily sweep at daemon
  startup, same code path as `metrics.history_days`).

### Step 3 — Fixture library

Add under `bench/fixtures/` (new dir, not `tests/fixtures/` which is
for unit tests):

| File | Expected winner | Size target |
|---|---|---|
| `cargo_build_verbose.txt` | L1 (dedup of "Compiling X") | ~500 lines |
| `cargo_test_with_failures.txt` | L1 (strip passes, keep failures) | ~200 lines |
| `tsc_errors_node_modules.txt` | L2 (path shortening) | ~150 lines |
| `docker_logs_repetitive.txt` | L1 (massive dedup) | ~400 lines |
| `generic_long_log.txt` | L3 (semantic) | >300 tokens |
| `already_short.txt` | below `MinChars`, hook skips | <500 chars |
| `git_diff_large.txt` | L2 (token-aware) | ~500 lines |
| `stack_trace_java.txt` | L3 (structural summary) | ~60 lines deep |

Each fixture has a sibling `<name>.meta.json`:

```json
{ "category": "build", "expected_layer": 1, "min_ratio": 0.7 }
```

### Step 4 — Microbench script

**File:** `bench/replay.sh`

```bash
#!/bin/sh
# Replay every fixture through the live daemon and write a CSV row.
for fx in bench/fixtures/*.txt; do
  name=$(basename "$fx" .txt)
  payload=$(jq -Rs --arg cmd "$(cat $fx.meta.json | jq -r .command // \"unknown\")" \
    '{output: ., command: $cmd, cwd: "/"}' < "$fx")
  t0=$(date +%s%N)
  resp=$(curl -sf --max-time 120 -X POST \
    http://127.0.0.1:8765/compress \
    -H 'Content-Type: application/json' \
    -d "$payload")
  t1=$(date +%s%N)
  latency=$(( (t1 - t0) / 1000000 ))
  echo "$name,$resp,$latency" >> microbench.csv
done
```

Output CSV columns: `fixture, bytes_in, tokens_before, tokens_after_l1,
tokens_after_l2, tokens_after_l3, tokens_after, layer_used, ratio,
latency_ms_total, latency_ms_l1, latency_ms_l2, latency_ms_l3`.

### Step 5 — Fixed test prompt (`bench/prompts/baseline.md`)

The prompt must be deterministic, trigger many Bash tools with large
outputs, and not depend on network state.

```markdown
You are running inside the NTK repo (`pwd` should end in `/ntk`).
Run the following commands **in order**, one per tool call, and wait
for each to finish before starting the next. Do NOT summarize until
step 8.

1. `cargo build --release --verbose 2>&1 | head -400`
2. `cargo test --no-run --verbose 2>&1 | head -200`
3. `git log --stat --format=fuller -30 2>&1`
4. `find src -name "*.rs" -exec wc -l {} \; 2>&1 | sort -rn | head -30`
5. `cargo tree --edges normal --prefix depth 2>&1 | head -300`
6. `cargo clippy --release -- -W clippy::pedantic 2>&1 | head -300`
7. `ls -laR src/ 2>&1 | head -400`
8. Now summarize: how many Rust files, total LOC, top-3 largest
   modules, and whether clippy reported warnings.
```

Why this prompt:

- 7 distinct Bash calls — enough sample size per run.
- Mix of structured (cargo) and unstructured (ls, find) outputs.
- Outputs range from ~200 to ~1000+ lines — covers all three layers.
- Step 8 forces Claude to actually **read** the compressed outputs
  (not just skip), so the end-to-end path is exercised.
- Deterministic when run in the same repo at the same commit.

### Step 6 — Session runner & parser

**File:** `bench/session.sh`

```sh
# Variant A: hook disabled
ntk init --uninstall
claude -p "$(cat bench/prompts/baseline.md)" \
  --output-format stream-json > transcripts/A.jsonl

# Variant B: hook enabled + logging enabled
ntk init -g
NTK_LOG_COMPRESSIONS=1 ntk start &
claude -p "$(cat bench/prompts/baseline.md)" \
  --output-format stream-json > transcripts/B.jsonl

# Parse both
python3 bench/parse_transcript.py transcripts/A.jsonl > A.csv
python3 bench/parse_transcript.py transcripts/B.jsonl > B.csv
```

**`bench/parse_transcript.py`** reads JSONL, sums `message.usage`
fields per turn, produces CSV with `turn, input_tokens,
cache_read_input_tokens, cache_creation_input_tokens, output_tokens,
total_tokens`.

Whether `claude -p` exists as a non-interactive mode needs to be
confirmed — see *Open questions* below.

### Step 7 — Report generator

**File:** `bench/report.py`

Reads `microbench.csv`, `A.csv`, `B.csv`, `~/.ntk/logs/...` and emits
`report.md` with:

- **Table 1:** per-fixture compression ratio (microbench).
- **Table 2:** A-vs-B session totals (input tokens, output tokens,
  cache hits, estimated cost in USD — rates configurable).
- **Table 3:** where the tokens were removed — grouped by fixture
  category, showing L1/L2/L3 contribution %.
- **Chart (ASCII):** histogram of ratios per layer.
- **Flags:** fixtures where ratio < 20% (NTK barely helped), ratios
  > 90% (NTK was worth the latency), ratios > 95% (check for
  information loss manually).

---

## How to actually run the test (suggested workflow for the user)

```sh
# Prerequisites
cargo build --release                 # v0.2.24+
export NTK_LOG_COMPRESSIONS=1

# 1. Micro: validate compressor math
ntk start &
bash bench/replay.sh                  # writes microbench.csv

# 2. Macro baseline: without hook
ntk init --uninstall
# restart Claude Code (hooks loaded at session start)
# manually paste bench/prompts/baseline.md
# save transcript: cp ~/.claude/projects/<proj>/<session>.jsonl transcripts/A.jsonl

# 3. Macro with hook
ntk init -g
ntk start                             # fresh session so metrics reset
# restart Claude Code again
# paste bench/prompts/baseline.md
# save transcript: cp ~/.claude/projects/<proj>/<session>.jsonl transcripts/B.jsonl

# 4. Report
python3 bench/report.py \
  --micro microbench.csv \
  --a transcripts/A.jsonl \
  --b transcripts/B.jsonl \
  --logs ~/.ntk/logs/ \
  --out report.md
```

---

## Open questions — must be decided before implementation

1. **`save_compressions` default** — off (privacy, disk churn) or on
   (friction-free benchmarking)? Recommendation: off, enable via env
   var only.

2. **`claude -p` non-interactive mode** — does it exist in the
   installed Claude Code version? If not, the macrobench has to be
   run manually (user pastes prompt twice, closes Claude between
   runs). Verify with `claude --help`.

3. **Cost reporting** — tokens only, or convert to USD using
   hardcoded Sonnet 4.6 rates (`$3 / 1M input, $15 / 1M output`,
   cache hits at `$0.30 / 1M`)? Rates change — if USD, read from a
   config file so they can be updated without a code release.

4. **Report scope** — raw CSV only (user analyzes), or also rendered
   markdown with conclusions? If rendered, picking "NTK helped" vs
   "NTK hurt" requires a threshold — what's a fair cutoff?
   (Proposal: net savings < 10% of tokens-before = "marginal";
   < 0 = "overhead without benefit".)

5. **Should microbench fail CI?** If ratios regress below the
   `min_ratio` in `<fixture>.meta.json`, turn into a release gate?
   Or keep purely informational?

---

## Expected effort per step

| Step | What changes | Effort |
|---|---|---|
| 1. Per-layer metrics in response | `src/server.rs`, `src/compressor/mod.rs` | ~1 h |
| 2. Opt-in persistence | Config + handler | ~1 h |
| 3. Fixture library | 8 new `.txt` + `.meta.json` | ~1 h |
| 4. `replay.sh` + CSV | Shell script | ~30 min |
| 5. `baseline.md` | Prose | 15 min |
| 6. Session runner + parser | Shell + Python | ~1.5 h |
| 7. Report generator | Python | ~2 h |
| **Total** | | **~7 h** of focused work |

Steps 1 and 2 gate everything else (need the data first). Steps 3-5
can be written in parallel with 1-2. Steps 6-7 come last.

---

## Decision point

Close open questions 1-4 (question 5 can be deferred), then
implementation proceeds in the order above. Each step is a separate
commit / PR so regressions are bisectable.
