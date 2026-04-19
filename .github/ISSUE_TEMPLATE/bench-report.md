---
name: Benchmark report
about: Submit benchmark numbers from `ntk bench --submit` on your hardware
title: "bench: <OS / CPU / GPU> results"
labels: ["testing", "performance", "help wanted"]
---

Thanks for running the bench! These numbers let maintainers set realistic
expectations per hardware class and catch regressions on platforms we
don't own directly.

## How you generated this

```bash
ntk bench --submit --output bench-report.json
# then paste the contents of bench-report.json below
```

## Report JSON

Paste the full content of `bench-report.json` here:

```json
<paste here>
```

## Hardware notes (free form)

- CPU model:
- GPU model (if any):
- RAM:
- OS version:
- Anything noteworthy (thermal throttling, battery power, etc.):

## Reproducibility

- [ ] Ran `ntk bench --submit` at least twice with similar numbers
- [ ] Daemon was idle (no other compression requests) during the bench
- [ ] Ollama / Candle / llama.cpp model was fully loaded (warm) before the run
