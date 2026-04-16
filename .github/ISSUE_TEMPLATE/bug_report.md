---
name: Bug report
about: Something NTK is doing wrong — wrong compression, crash, lost error line, hook failure.
title: "[bug] <short summary>"
labels: bug
assignees: ''
---

## What happened

<!-- One or two sentences. "After upgrading to v0.2.29, NTK drops the
error line from my cargo test output." -->

## Steps to reproduce

1. …
2. …
3. …

If the bug is triggered by a specific command output, attach it to the
issue (drag-drop the `.txt` or paste it fenced). Smallest reproducer
wins.

## What you expected

<!-- "The line `error[E0382]: borrow of moved value` should survive L1." -->

## What actually happened

<!-- Paste the compressed output, or the panic, or the HTTP status. -->

## Environment

Run `ntk status` and paste the output here, **or** fill the table below:

| | |
|---|---|
| NTK version (`ntk --version`) | |
| OS + version | |
| Editor (Claude Code / OpenCode / other) | |
| Daemon backend (`ollama` / `candle` / `llama_cpp`) | |
| GPU vendor + model | |
| `gpu_layers` setting | |

## Config file

If relevant, paste `~/.ntk/config.json` (or the project's `.ntk.json`),
with secrets redacted.

```json

```

## Additional context

<!-- Anything else: related issues, workarounds you tried, screenshots. -->
