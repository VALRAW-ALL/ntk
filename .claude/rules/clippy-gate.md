# Rule: NTK Clippy Security Gate

Applies to: any Rust code change in the NTK project (`src/`, `examples/`, `tests/`).

## Before committing Rust code, run locally

```bash
cargo fmt --check
cargo clippy -- \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic \
  -W clippy::arithmetic_side_effects \
  -D warnings
```

These are the **exact flags CI uses**. If they pass locally they will pass in CI.

## Common violations introduced in the past

### arithmetic_side_effects
Applies to `usize` / integer arithmetic. Use saturating methods:
```rust
// BAD  — triggers warning
let num = i + 2;
let max = 1 + gpus.len();
let idx = choice - 2;
idx += 1;

// GOOD
let num = i.saturating_add(2);
let max = gpus.len().saturating_add(1);
let idx = choice.saturating_sub(2);
idx = idx.saturating_add(1);
```

### unwrap_used
Never use `.unwrap()` on `Option` or `Result` in production code (`src/`).
Use `if let`, `match`, or `?`. Exception: test code in `tests/`.

```rust
// BAD
let chosen = gpus.get(id).or_else(|| gpus.first()).unwrap();

// GOOD
if let Some(chosen) = gpus.get(id).or_else(|| gpus.first()) { ... }
```

## Important: clippy runs on lib+bin, NOT tests by default

The CI command has no `--all-targets` flag, so it only checks the library and binary targets. Tests can use `.unwrap()` freely. Do not add `--all-targets` to the CI command or you will need to fix all test code too.

## Linux vs Windows clippy differences

Linux Rust may catch additional warnings not caught on Windows. Always validate with Docker if changing `src/gpu.rs`:
```bash
docker run --rm -v "$(pwd -W)":/src -w /src rust:1-bookworm bash -ec '
  rustup component add clippy
  cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used \
    -W clippy::panic -W clippy::arithmetic_side_effects -D warnings
'
```

## Toolchain drift — run `rustup update stable` before clippy

CI uses `dtolnay/rust-toolchain@stable` which always resolves to the
**latest** stable. If your local stable is a release behind, clippy will
miss lints introduced in the interim and you'll learn about them only
after pushing.

Before committing Rust changes:
```bash
rustup update stable
cargo clippy -- <the security flags above>
```

### Known lints introduced per release

| Release | Lint added | Fix pattern |
|---|---|---|
| 1.95.0 | `clippy::unnecessary_sort_by` | `sort_by(|a,b| b.1.cmp(&a.1))` → `sort_by_key(|e| std::cmp::Reverse(e.1))` |

Append rows when CI catches a new lint so future contributors get the
fix recipe without re-diagnosing.
