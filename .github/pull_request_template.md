<!-- Thanks for contributing to NTK! A couple of quick things that help
reviews land fast — feel free to delete sections that don't apply. -->

## What this PR does

<!-- 1-2 sentence summary. "Adds L1 stack-trace classifier for
Elixir/Phoenix with 2 new fixtures and one in-module test." -->

## Why

<!-- What problem does this solve? Link the issue if there is one. -->

Closes #

## Checklist

Project-wide conventions — tick the ones that apply:

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -W clippy::arithmetic_side_effects -D warnings` passes (the CI gate, see [`.claude/rules/clippy-gate.md`](../.claude/rules/clippy-gate.md))
- [ ] `cargo test` passes locally on my machine
- [ ] I ran `rustup update stable` before testing (catches lints added in newer Rust)
- [ ] If this touches L1/L2 algorithms, the proptest suite (`cargo test --test compression_invariants`) still passes
- [ ] If I added a new stack-trace language, I followed [`.claude/skills/add-stack-trace-language.md`](../.claude/skills/add-stack-trace-language.md) (fixture + meta + generator + test + docs row)
- [ ] If I changed snapshot output intentionally, I re-generated with `INSTA_UPDATE=always cargo test --test snapshot_tests` and reviewed the diff
- [ ] I did **not** add fake benchmark numbers to the docs (use `N/A` until measured)

## Screenshots / before-after (optional)

<!-- For docs, UI, or compression-ratio changes. -->

## Additional notes

<!-- Anything reviewers should know about trade-offs, alternatives
considered, follow-up work. -->
