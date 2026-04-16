# Skill: bench-runner

Run the NTK benchmark harness to validate compression ratios after code
changes. Invoke before any PR that touches `src/compressor/` or after adding
new fixtures.

## Steps

### 1. Quick microbench (L1+L2 only, fast)

```powershell
# Windows — set a low L3 timeout so each fixture takes < 10s total:
$env:NTK_LOG_COMPRESSIONS = '1'
ntk stop
ntk start

# Lower timeout temporarily so L3 always falls back:
# Edit ~/.ntk/config.json: "timeout_ms": 5000
pwsh bench/replay.ps1 -TimeoutSec 10
pwsh bench/report.ps1
```

```bash
# Unix
NTK_LOG_COMPRESSIONS=1 ntk start &
TIMEOUT_SEC=10 bash bench/run_all.sh
```

### 2. Full run (includes L3 inference — slow on CPU)

```powershell
pwsh bench/run_all.ps1 -TimeoutSec 300
```

### 3. Prompt format A/B (L4)

```powershell
pwsh bench/prompt_formats.ps1
```

### 4. End-to-end Claude Code A/B

```powershell
pwsh bench/ab_session.ps1
```

## Minimum acceptable ratios (from bench/fixtures/*.meta.json)

| Fixture | Min ratio |
|---|---:|
| docker_logs_repetitive | 50% |
| cargo_test_failures | 30% |
| stack_trace_java | 40% |
| node_express_trace | 50% |
| python_django_trace | 30% |
| go_panic_trace | 40% |
| php_symfony_trace | 20% |

If any fixture falls below its `min_ratio`, investigate before merging.

## Where results go

- `bench/microbench.csv` — per-fixture token counts
- `bench/report.md` — rendered markdown
- `~/.ntk/logs/YYYY-MM-DD/*.json` — per-compression trace (when
  `NTK_LOG_COMPRESSIONS=1` set)
- `bench/prompt_formats.csv` — L4 format A/B
- `bench/transcripts/{A,B}.{jsonl,csv}` — Claude Code A/B macro

All outputs except fixtures are gitignored.

## Updating snapshots after intentional L1/L2 changes

```bash
INSTA_UPDATE=always cargo test --test snapshot_tests
git add tests/integration/snapshots/
```
