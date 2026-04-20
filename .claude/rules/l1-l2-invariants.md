# Rule: L1/L2 Compression Invariants

Applies to: any change to `src/compressor/layer1_filter.rs`,
`src/compressor/layer2_tokenizer.rs`, or any new pipeline stage that sits
between the daemon and Layer 3.

## Non-negotiable invariants

The following properties **must** hold after every L1/L2 change. CI will
catch most via the snapshot tests, but some need dedicated unit tests.

1. **No loss of error/warning lines.** Any line containing `ERROR`, `error:`,
   `FAILED`, `panic:`, `Caused by`, `Traceback`, `Exception`, `fatal`,
   `E0[0-9]{3}:` or `warning:` survives to the final output. Error info
   is the signal NTK is paid to preserve.
2. **At least one exemplar per deduplicated template.** When template
   dedup collapses N lines into `[×N] <line>`, the exemplar must be the
   actual first occurrence, not a synthesized placeholder. The user must
   be able to read a real line to understand what was grouped.
3. **First and last stack frame preserved.** When the stack-trace filter
   collapses framework frames, the first framework frame (the entry point
   the user's code transitioned into framework land) must stay. User
   frames must never be filtered, regardless of their indentation.
4. **Deterministic output.** Running `filter()` twice on the same input
   yields byte-identical output. No wall-clock time, random IDs, or
   iteration order of unordered collections in the output.
5. **Idempotent.** `filter(filter(x).output)` produces the same string as
   `filter(x).output`. Re-running the pipeline on already-compressed
   output must not degrade it further.

## When adding a new normalization pattern

- Add a regex to the appropriate `Lazy<Regex>` block with
  `#[allow(clippy::expect_used)]` — the literal is caught at compile time
  by the test suite, so `.expect("... regex must compile")` is safe.
- Add at least two unit tests:
  1. **Positive**: an input where the pattern fires, with assertions on
     the output.
  2. **Negative**: an input that *looks* similar but must NOT be touched
     (e.g. short hex like git SHAs, version strings, etc).
- Run `cargo test --lib compressor` locally before commit — L1 has 25
  tests, L2 has 12, L4 has 10. If you add a new pattern, bump the totals.

## L1 stage order (do NOT reorder casually)

The order below is load-bearing. Moving stages up/down changes what
the next stage sees and can break idempotency (invariant #5) or the
first/last-frame guarantees (#3).

1. `strip_ansi` — must run first; everything else assumes plain text.
2. `detect_rtk_output` — sets `rtk_pre_filtered` for metrics only.
3. `remove_progress_bars` — also drops cargo/rustc status lines
   (`   Compiling <crate> v<N>.<N>.<N>`, `Checking`, `Building`,
   `Fresh`, `Installing`, `Updating`) via `is_cargo_progress()`.
   The final `Finished` verdict line intentionally stays.
4. `remove_diagnostic_noise` — drops TS/Rust/Elm wavy underline rows
   (`~~~~~~^^^`) and git porcelain per-file metadata (`index abc..def`,
   `--- a/path`, `+++ b/path`). Line-conservative: any non-noise
   content on the line keeps it alive.
5. `group_by_template` — template-dedup runs AFTER diagnostic noise
   so the template pool isn't polluted by wavy rows and index SHAs.
6. `filter_stack_frames` — stack-trace run collapse.
7. `factor_common_prefix` — `[prefix: …]` summarization.
8. `filter_test_failures` — extract FAILED blocks when present.
9. `collapse_blank_lines` — always last; never interleave with dedup.

## When modifying stack-trace filter

Multi-language matrix — must pass after any change:

| Language | Fixture | Min ratio |
|---|---|---:|
| Java | `stack_trace_java` | 50% |
| Python | `python_django_trace` | 50% |
| Go | `go_panic_trace` | 40% |
| Node | `node_express_trace` | 70% |
| PHP | `php_symfony_trace` | 20% |
| Ruby | `ruby_rails_trace` | 5% (spec_loader adds the real win) |
| Elixir | `elixir_phoenix_trace` | 5% (spec_loader) |
| Swift | `swift_uikit_crash` | 0% (spec_loader only) |
| kubectl | `kubectl_describe_pod` | 0% (spec_loader only) |

The last four fixtures have min_ratio set low on purpose: L1
hardcoded has no coverage for those ecosystems, so the real
compression is asserted by `tests/integration/spec_corpus_integration.rs`
spot-checks (`*_ruleset_collapses_on_*_fixture`) rather than
`bench_ratios_regression`. Do NOT raise those min_ratio values
unless you also port the language into `filter_stack_frames`.

Run `pwsh bench/replay.ps1 -TimeoutSec 10` to confirm none regresses
below the `min_ratio` in the corresponding `.meta.json`. Adjust the
classifier tables (Spring, Django, Rails, Express, runtime.*, etc.)
rather than relaxing the test — the fixtures ARE the contract.

## Performance budget

Full L1+L2 pipeline on 10 000-line input: < 50 ms on a 2022-era laptop
CPU. If you add a stage that iterates over lines more than once, profile
with `cargo bench` before merging.
