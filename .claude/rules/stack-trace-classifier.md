# Rule: Stack-Trace Classifier Extension

Applies to: any change to `is_framework_frame`, `is_framework_frame_unit`,
`is_python_frame_body`, `is_go_frame_body` in
`src/compressor/layer1_filter.rs`.

## Architecture

The stack-trace filter works on **runs** of framework frames. A line is
classified as framework by `is_framework_frame`, then `filter_stack_frames`
collapses runs of ≥ 3 consecutive framework units into
`[N framework frames omitted]`. User code is always preserved.

### Classifier conventions

Use `starts_with` for language-specific frame prefixes. Use `contains`
for package-scoped markers that can appear anywhere in the line:

```rust
// GOOD — "at " + strict prefix is a reliable marker for that ecosystem
if t.starts_with("at org.springframework.") { return true; }
if t.starts_with("at Microsoft.AspNetCore.") { return true; }

// GOOD — path marker for Node / Python / PHP
if t.contains("node_modules/react-dom/") { return true; }
if t.contains("/site-packages/")         { return true; }
if t.contains("/vendor/symfony/")        { return true; }
```

### User-entrypoint exclusions (invariant #3)

Some patterns would otherwise swallow the user's own entrypoint. Always
exclude them explicitly:

```rust
// Go: main.main is user code — runtime.* is framework, but main.* is not
if t.starts_with("main.main(") { return false; }
```

If you find yourself adding a language where the user's code lives in a
namespace that overlaps with framework (e.g. a user's package named
`androidx.myapp`), document the exclusion here.

## Frame-body pairing (2-line units)

Some languages emit a frame as a **pair** of lines (header + indented
body). The classifier handles these via `is_framework_frame_unit` which
returns `(is_framework, unit_len)`:

| Language | Header | Body |
|---|---|---|
| Python | `  File "/path/foo.py", line 42, in fn` | `    actual_source_code()` |
| Go | `runtime.goexit()` | `\t\t/usr/local/go/src/runtime/asm.s:1650 +0x5` |

When adding a new language that emits multi-line frames, extend
`is_framework_frame_unit` with the pairing rule and bump `unit_len`
accordingly. Do NOT add the body line to `is_framework_frame` directly —
a body line in isolation (user code that happens to be indented) would
be misclassified.

## Required testing per new language

Adding a language means adding **all four** of:

1. In-module unit test in `src/compressor/layer1_filter.rs`:
   `test_stack_trace_<language>_filter` — verify error message + at
   least one user frame survive, framework frames collapse.
2. External fixture in `bench/fixtures/<language>_<framework>_trace.txt`
   + sibling `.meta.json` with `category=stack_trace`, `min_ratio`, etc.
3. Entry in `bench/generate_fixtures.ps1` that reproduces the fixture
   so a clean run regenerates the whole library.
4. Row in the docs metrics table
   (`docs/index.html` + translation keys in `docs/app.js`).

## min_ratio guidance

The `min_ratio` field in each fixture's `.meta.json` encodes the
minimum acceptable L1+L2 compression. Use these starting values and
tighten only after you've confirmed the filter is deterministic:

| Frame density | Suggested min_ratio |
|---|---:|
| Deep framework trace (Java/Spring, Node/Express) | 0.50 – 0.70 |
| Shallow user-heavy trace (Python/Django)        | 0.30 – 0.45 |
| Mixed runtime dumps (Go panic, Kotlin Android)  | 0.35 – 0.45 |
| Loose vendor frames (PHP Symfony)               | 0.30 – 0.40 |

Do not lower a fixture's min_ratio to paper over a regression. The
fixture IS the contract — if compression worsens, extend the classifier.

## Languages currently supported

Java (Spring/Tomcat/CGLIB), Python (Django/site-packages/gunicorn/
asgiref/werkzeug), Ruby (actionpack/activesupport/railties/rack),
Node.js (Express/node:internal), Go (runtime.*), PHP
(Symfony/Laravel/Illuminate), Rust (core::panicking / std::panicking),
.NET (Microsoft.AspNetCore / System.Threading.Tasks /
System.Runtime.ExceptionServices), JavaScript/TypeScript browser
(webpack / react-dom / zone.js), React Native (metro / Libraries),
Kotlin/Android (androidx / kotlinx.coroutines / com.android / dalvik).
