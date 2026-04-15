# NTK Benchmark — Baseline Prompt

> Deterministic prompt used to measure end-to-end token consumption
> with and without the NTK hook. Paste this verbatim into Claude Code
> (in a fresh session, working dir = NTK repo root).
>
> **Do not run any commands before pasting** — the A/B comparison
> relies on both runs starting from the same empty session state.

---

You are running inside the NTK repository (`pwd` should end in `/ntk`).
I want you to run the following Bash commands **in order**, one per
tool call, waiting for each to complete before starting the next.
Do NOT summarize or comment until step 8.

1. `cargo build --release --verbose 2>&1 | head -400`
2. `cargo test --no-run --verbose 2>&1 | head -200`
3. `git log --stat --format=fuller -30 2>&1`
4. `find src -name "*.rs" -exec wc -l {} \; 2>&1 | sort -rn | head -30`
5. `cargo tree --edges normal --prefix depth 2>&1 | head -300`
6. `cargo clippy --release -- -W clippy::pedantic 2>&1 | head -300`
7. `ls -laR src/ 2>&1 | head -400`
8. Now summarize in 3 short bullets:
   - How many Rust files are in `src/` and total LOC.
   - Top-3 largest modules (by line count).
   - Whether clippy reported any warnings, and if so, the 2 most
     common categories.

Do not ask clarifying questions. Run the commands as listed.
