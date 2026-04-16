# Contributing to NTK

NTK (Neural Token Killer) is an **open initiative**. It was started by
one person and the surface area has outgrown what one person can cover
well. If you've ever opened a PR on a Rust project, or written a shell
script, or staged a 50-MB log for a postmortem — your time can measurably
improve this project.

## Ways to help, sorted by "easiest to land"

### Starter tasks (no Rust required)

1. **Test NTK in your own workflow** and open an issue with your
   `bench/microbench.csv` output. Real-world compression ratios are the
   single most useful data we can collect right now.
2. **Add a fixture** under `bench/fixtures/`: capture the output of a
   tool or language we don't cover yet (Ruby, Elixir, Scala, Haskell,
   Swift, Flutter, Terraform, Ansible, …). See
   `.claude/skills/add-stack-trace-language.md` for the exact steps —
   each fixture is one `.txt` + one `.meta.json`.
3. **Translate the docs site**. `docs/app.js` already has EN+PT blocks;
   adding ES / FR / DE / JA is a pure copy-then-translate task.

### Medium tasks (Rust, low risk)

4. **Extend the stack-trace classifier** with a new language. Concrete
   playbook in `.claude/skills/add-stack-trace-language.md`. Currently
   supported: Java/Spring, Python/Django, Ruby/Rails, Node/Express,
   Go, PHP/Symfony, Rust, .NET/ASP.NET Core, JS/TS browser (React), React
   Native, Kotlin/Android. Still open: **Erlang/OTP**, **Scala/Akka**,
   **Elixir/Phoenix**, **Swift/iOS**, **Flutter/Dart**, **Clojure**.
5. **Add L1 patterns** for repetitive output formats we don't dedup yet
   (Terraform plans, ansible-playbook, kubectl apply, npm install
   warnings, apt-get output). See `l1_filter.rs` + proptest suite.
6. **Port the hook** to another editor. We ship `scripts/ntk-hook.sh`
   and `scripts/ntk-hook.ps1` for Claude Code / OpenCode. Cursor,
   Aider, Zed, Continue, and Windsurf all use similar tool-output
   hooks — a fresh copy of the hook with their JSON schema is a small
   self-contained PR.

### Hard tasks (deep context)

7. **Real-hardware GPU benchmarks**. The README / docs claim
   p50/p95 numbers for AMD (via Vulkan/llama.cpp), Apple Metal, Intel
   AMX, and NVIDIA across the RTX 30/40/50 series. Only NVIDIA 3060 +
   Intel i5 numbers are directly measured; the rest are estimates from
   public benchmarks. If you own the hardware, run `ntk model bench`
   and open a PR replacing the estimate with your measurement.
8. **L3 prompt A/B**. Our current prompts (`bench/prompts/*.txt`) are a
   first pass. A/B experiments comparing prompt formats
   (Prefix/XmlWrap/Goal/Json) can be run with
   `bench/prompt_formats.ps1`. Better prompts = better compression.
9. **Property-based tests**. `tests/proptest/compression_invariants.rs`
   has 8 invariants. If you find an input that produces a misleading
   output (wrong exemplar, lost error line, non-idempotent behaviour),
   that's an invariant we should encode.

## Before you submit a PR

```bash
# Formatting & lint gate — these are the EXACT flags CI uses
cargo fmt --check
cargo clippy -- \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic \
  -W clippy::arithmetic_side_effects \
  -D warnings

# Full test suite
cargo test

# If you changed L1/L2 algorithms, re-run the proptest invariants
cargo test --test compression_invariants

# If you changed snapshots intentionally
INSTA_UPDATE=always cargo test --test snapshot_tests
```

If the clippy gate passes locally, it will pass in CI. If you hit a
snapshot failure, look at the diff first — if the change is intentional,
review and commit the new `.snap` files.

## Code of conduct

Be kind. Assume good intent. Critique ideas, not people. If you can't
write a review comment that a stranger would read as constructive, sleep
on it and try again the next day.

## Licensing

NTK is released under the MIT License. By submitting a PR you agree
that your contribution is offered under the same license.

---

**TL;DR** — if any of the above speaks to you, pick the smallest task
you can finish end-to-end this weekend and open a PR. Don't wait for
permission; the starter tasks are pre-scoped for a reason.
