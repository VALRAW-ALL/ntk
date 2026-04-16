# NTK Benchmark Harness

Measure how many tokens NTK actually saves. See the full plan in
[`../docs/testing-plan.md`](../docs/testing-plan.md) (EN) or
[`../docs/plano-de-testes.md`](../docs/plano-de-testes.md) (PT-BR).

## One-shot runner

```powershell
# Requires: ntk on PATH, NTK daemon compiled with v0.2.24+
pwsh bench/run_all.ps1
```

This will:

1. Generate fixtures (only if missing).
2. Restart `ntk` with `NTK_LOG_COMPRESSIONS=1` so every compression is
   persisted to `~/.ntk/logs/YYYY-MM-DD/`.
3. Run `replay.ps1` — posts each fixture to `/compress` and writes
   `microbench.csv`.
4. Run `report.ps1` — produces `report.md` with per-fixture table,
   flags, and (if A/B CSVs provided) session comparison.

## Individual scripts

| Script | Purpose |
|---|---|
| `generate_fixtures.ps1` | Creates the 12 `.txt` + `.meta.json` fixture pairs. Idempotent. |
| `replay.ps1` / `replay.sh` | Loops fixtures, calls `/compress`, writes CSV. Shell version available for Unix. Pass `-Context "<intent>"` (PS) or `NTK_BENCH_CONTEXT=...` (sh) to also exercise Layer 4. |
| `parse_transcript.ps1` | Parses a Claude Code session `.jsonl` into a per-turn token CSV. |
| `report.ps1` | Aggregates microbench + A/B CSVs into `report.md`. |
| `run_all.ps1` | Orchestrator calling the three above. |

## Fixtures

8 deterministic fixtures covering the full range of compression
scenarios — ratios and expected layers are documented in each
`.meta.json`:

| Fixture | Expected layer | What it stresses |
|---|---|---|
| `already_short` | L0 / skip | Input below hook's 500-char floor |
| `cargo_build_verbose` | L1/L3 | Many `Compiling X` lines → dedup |
| `cargo_test_failures` | L1/L2 | Keep failures, drop passing tests |
| `tsc_errors_node_modules` | L2 | Long paths → tokenizer shortening |
| `docker_logs_repetitive` | L1 | Massive repeat → dedup dominates |
| `generic_long_log` | L3 | Unstructured prose → needs semantic summary |
| `git_diff_large` | L2 | Structure-preserving compression |
| `stack_trace_java` | L3 | Deep stack trace → root cause summary |

## Macrobench (A/B session comparison)

1. Copy the prompt from `bench/prompts/baseline.md`.
2. Run Claude Code **without** the NTK hook:
   ```powershell
   ntk init --uninstall
   # restart Claude Code, paste prompt, save transcript:
   cp ~/.claude/projects/<proj>/<session-id>.jsonl bench/transcripts/A.jsonl
   ```
3. Run Claude Code **with** the NTK hook:
   ```powershell
   ntk init -g
   # restart Claude Code (hooks load on session start)
   $env:NTK_LOG_COMPRESSIONS = "1"
   ntk start
   # paste prompt, save transcript:
   cp ~/.claude/projects/<proj>/<session-id>.jsonl bench/transcripts/B.jsonl
   ```
4. Parse + report:
   ```powershell
   pwsh bench/parse_transcript.ps1 -Transcript bench/transcripts/A.jsonl
   pwsh bench/parse_transcript.ps1 -Transcript bench/transcripts/B.jsonl
   pwsh bench/report.ps1 `
     -A bench/transcripts/A.csv `
     -B bench/transcripts/B.csv
   ```

The report's "Estimated cost (Sonnet 4.6)" line converts tokens to
USD using the default rates (input $3/1M, output $15/1M, cache reads
$0.3/1M, cache writes $3.75/1M). Override via script flags.

## Outputs

- `microbench.csv` — one row per fixture + daemon response.
- `~/.ntk/logs/YYYY-MM-DD/*.json` — full compression trace per call.
- `bench/transcripts/{A,B}.{jsonl,csv}` — Claude Code session data.
- `bench/report.md` — human-readable report.
