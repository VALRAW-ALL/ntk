---
name: New stack-trace language support
about: Request or propose L1 filter coverage for a language / framework we don't handle yet.
title: "[stack-trace] <language / framework>"
labels: enhancement, stack-trace
assignees: ''
---

## Language / framework

<!-- e.g. "Elixir / Phoenix", "Swift / iOS", "Scala / Akka" -->

## A real stack trace

Paste an actual stack trace — 15–30 lines is ideal. Keep it real: made-up
traces produce classifier rules that don't generalise.

```

```

## What marks a framework frame?

<!-- Look at the paste and tell us the *pattern* that distinguishes
framework / runtime frames from the user's own code. Examples from
existing support:

- Java / Spring: `at org.springframework.` prefix
- Python / Django: `File "…/site-packages/django/…"` path marker
- Go: `runtime.*` prefix + `/usr/local/go/src/runtime/` body path
- PHP / Symfony: `#0 /app/vendor/symfony/` prefix

Your answer here shapes the classifier patch. -->

## User entrypoint

<!-- Is there an ambiguity where user code could be mis-classified as
framework? E.g. in Go, `runtime.*` is framework but `main.main` is user
code. List any such cases so we add the right `return false` exclusion. -->

## Willing to contribute?

- [ ] I can send a PR following `.claude/skills/add-stack-trace-language.md`
- [ ] I can only provide the trace — someone else should write the patch
- [ ] I want to pair on it (comment below)

## Additional context

<!-- Links to language docs, other tools that tackle the same problem,
real-world log volume where this would matter. -->
