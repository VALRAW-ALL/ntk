# RFC-0001 — Context Linter Spec

**Status:** draft (POC shipped on branch `poc/context-linter-spec`) · **Authors:** NTK maintainers + community
**Supersedes:** —
**Obsoletes:** —

**POC status (2026-04-19):**
- Rust reference impl: `src/compressor/spec_loader.rs` (all 4 primitives, 10 unit tests passing)
- JS second binding: `bindings/ctxlint-js/` (9 parity tests passing)
- Bench vs hardcoded: **4.5× faster** on Python fixture (28 µs vs 127 µs) — well inside the +20 % kill criterion from §16 step 2
- Gate for publishing to the wider community still stands: ≥100 stars or ≥5 external contributors before opening the 30-day comment window

> **Reading note.** This RFC is the foundation for issues #23, #24,
> #25, #26. It is published as a design document only — the gate for
> turning it into an accepted RFC with community sign-off is the one
> stated in issue #23: **~100 GitHub stars OR ≥ 5 active external
> contributors** before the 30-day comment window opens. Until then
> the document is iterated by the maintainers and no breaking changes
> are made to the schema.

## 1. Motivation

NTK's Layer 1 compresses LLM-agent command output by applying a
hand-written mix of regexes, template-dedup rules, and
stack-trace-frame classifiers. The rules work. They also live as
Rust code — every new language or framework requires a PR against
`src/compressor/layer1_filter.rs`, a new in-module test, a bench
fixture, and a docs metric row.

That model does not scale. A one-file declarative spec that any
agent (Cursor, Cline, Aider, Continue, Zed, Windsurf, Claude Code)
can consume would let the ecosystem contribute rules as data, not
code. The analogy is ESLint: the win is **the shared rule format**,
not the specific rules.

The spec has two concrete goals:

1. **Make contribution trivial.** Adding Erlang or Elixir should be
   a YAML file plus a fixture, not a Rust PR.
2. **Decouple implementations.** NTK (Rust), a hypothetical
   `ctxlint-py`, a Cursor-native MCP server — all read the same
   file and produce the same compression.

A third, softer goal: establish **"context garbage" as a lintable
concept**, the same way "bad JS" became lintable in 2013 when ESLint
froze its rule format before anyone had written the good rules.

## 2. Scope — what the spec covers, what it doesn't

### In scope

- Deterministic text-to-text transformations (regex replace,
  line-level filter, run collapse, template dedup, prefix factor)
- Classifying lines as "framework / vendor / library" vs "user"
  for stack traces across **every mainstream language**
- Cross-platform path handling (Windows `\`, POSIX `/`, mixed
  quoting in shell output)
- Expressing the *intent* under which a rule should or should not
  fire (e.g., preserve the full stack trace when the user is
  debugging a test failure, collapse it aggressively when they
  are just running `cargo check`)
- Severity ordering so clients can opt into different aggression
  levels (`lossy-safe`, `lossy-risky`)

### Explicitly out of scope (for v1)

- Neural-inference rules (that's NTK's Layer 3 — it has its own
  prompt-format contract; a future RFC-0002 may bring L3 rules
  under the same umbrella, but the coupling is too tight today)
- Tokenizer-specific compressions (BPE path shortening etc —
  Layer 2 is BPE-aware, the spec is tokenizer-agnostic)
- Binary-output compression (NTK already rejects non-UTF-8
  aggressively; the spec assumes UTF-8 lossy-decoded text)
- Streaming semantics (rules operate on full-buffer input; future
  RFC can add `chunk_boundary` primitives if needed)

## 3. Reference artifacts surveyed

The rule taxonomy below is derived by inspecting real-world output
from the following languages/frameworks/tools. Each has at least one
committed fixture under `bench/fixtures/` or a documented pattern in
NTK's existing classifier.

| Language / runtime | Frameworks | Stack-trace shape | Progress / log shape |
|---|---|---|---|
| **Java / JVM** | Spring, Tomcat, CGLIB | `at org.springframework.X.m(File.java:N)` | Maven/Gradle `[INFO]` prefixes |
| **Python** | Django, Flask, FastAPI, gunicorn, asgiref, werkzeug | 2-line header/body: `File "/path/foo.py", line N, in fn` + indented source | pip wheel bars, pytest `.` dots, tqdm |
| **Ruby** | actionpack, activesupport, railties, rack, sidekiq | `file:line:in \`method'` one-liner | bundler install list |
| **Node.js / TypeScript** | Express, NestJS, Next, node:internal, webpack, react-dom, zone.js | `at fn (/path/file.js:N:N)` or `at Object.<anonymous> (...)` | npm progress, tsc dotted progress, webpack compilation bars |
| **React Native** | metro, Libraries/, RCTBridge | RN-tinted frames with `@` separator | metro bundle progress |
| **Go** | `runtime.*` machinery, stdlib | 2-line header/body: `func(args)` + `\t\t/path/file.go:N +offset` | `go test` `ok pkg 0.05s`, build spinners |
| **PHP** | Symfony, Laravel (Illuminate), Composer vendor | Three forms: `#N /path`, `at Symfony\\Class->m()`, `(/vendor/... )` inline | Composer dots, PHPUnit dots |
| **Rust** | `core::panicking`, `std::panicking`, `std::rt` | `std::backtrace::Backtrace` frames | cargo `Compiling crate v1.0.0` lines, criterion bars |
| **C# / .NET** | ASP.NET Core, System.Threading.Tasks, System.Runtime.ExceptionServices | `at Microsoft.AspNetCore.X.M(args)` | MSBuild `/out:`, dotnet CLI bars |
| **Kotlin / Android** | `androidx.*`, `kotlinx.coroutines`, `com.android`, dalvik | Dalvik 2-line + `Suppressed:` | Gradle daemon, Dexer progress |
| **Swift / iOS** | UIKit, Foundation, _dyld_start | Frame: `N   libfoo.dylib  0xADDR + OFFSET func + N` | xcodebuild `Compiling X.swift` |
| **Elixir / Erlang** | Phoenix, Ecto, OTP | `(app 1.0.0) lib/mod.ex:N: Module.fn/arity` | mix compile `==> app`, telemetry dots |
| **Haskell / Cabal** | stack, cabal | `CallStack (from HasCallStack): f, called at src/F.hs:N:C` | cabal `Configuring package`, dots |
| **Clojure** | Ring, Compojure | `at clojure.lang.Foo.invoke(Foo.java:N)` | lein/deps.edn jar resolve |
| **C / C++** | GNU/LLVM, libc, libstdc++ | `#N  0xADDR in func () from /lib/libc.so` | make `CC foo.c`, cmake `[ N%]` |
| **Docker / Podman** | — | N/A | `Step N/M`, `Pulling fs layer`, `Digest: sha256:…` |
| **Kubernetes** | — | N/A | `RESOURCE  STATUS  AGE` tables, `kubectl describe` repeated labels |
| **Git** | — | N/A | `remote: Counting objects:`, diff hunks `@@ -N,M +N,M @@` |
| **Windows PowerShell** | — | `At line:N char:N` + squiggly `~~~~` pointer | `Write-Progress` percent bars |
| **Windows cmd** | — | `CALL :label` trace | `XCOPY /M/E` per-file |
| **macOS Activity** | — | `Exception Type: EXC_*` + binary image table | Homebrew `==>` lines |
| **Linux systemd** | — | `Sep 10 ... systemd[1]: Started ...` one-liners | journalctl output |

This list is not exhaustive, but the shapes covered here already
reduce to **four transform primitives** (section 5). A language not
listed here should fit the same primitives; the burden of proof is
on the RFC to accept a new primitive only when a genuinely distinct
shape is presented.

## 4. Core concepts

### 4.1 Rule

A **rule** is a named unit of transformation. It has:

- `id`: stable identifier (`py.stack.framework`, `docker.layer_pull`)
- `applies_to`: which detector category the rule runs on (see 4.3)
- `pattern`: how lines/runs are matched (section 5)
- `transform`: what to do with the match (section 6)
- `severity`: `lossy-safe` | `lossy-risky` | `info` (section 7)
- `intent_scope`: optional user-intent gating (section 8)
- `invariants`: list of guarantees the rule must not violate (section 9)
- `metadata`: author, license, link to source, last-updated

### 4.2 Rule file

A `.yaml` or `.json` file containing a header + an array of rules.

```yaml
spec_version: 0.1
category: stack_trace
language: python
frameworks: [django, flask, gunicorn]
rules:
  - id: py.stack.site_packages_run
    applies_to: stack_trace.frame_run
    pattern:
      kind: frame-run
      classifier: contains
      values:
        - "/site-packages/"
        - "/gunicorn/"
        - "/asgiref/"
        - "/werkzeug/"
      unit: 2         # Python emits each frame as header + body line
    transform:
      kind: collapse-run
      min_run: 3
      replacement: "[{n} framework frames omitted]"
    severity: lossy-safe
    invariants: [preserve_first_frame, preserve_last_frame, preserve_errors]
    metadata:
      source: https://github.com/example/py-stack-rules
      license: MIT
```

### 4.3 Detector category

Before rules fire, the input is classified as one of:

- `stack_trace` — Java/Python/Go/etc trace
- `test_output` — pytest/jest/cargo-test/etc
- `build_log` — compiler/bundler output
- `container_log` — docker/k8s logs
- `diff` — git/patch output
- `generic` — anything else

Rules are tagged with the categories they apply to (most apply to
multiple). The category dispatcher is itself a thin ruleset — but
it lives in the spec's **core** and is not community-pluggable in
v1 (avoids detection-rule fights).

## 5. Rule patterns (the four primitives)

Every rule in the spec reduces to one of four patterns.

### 5.1 `line-match` — line-level filter/rewrite

Matches individual lines by regex or literal prefix/substring. The
simplest primitive; drives ANSI strip, progress-bar removal, and
timestamp normalization.

```yaml
pattern:
  kind: line-match
  classifier: regex        # regex | starts_with | contains | equals
  values: ["^\\s*\\[?[=#-]+>?\\s*\\d+%"]
transform:
  kind: delete             # delete | rewrite
```

### 5.2 `frame-run` — run of consecutive framework frames

The workhorse for stack traces. Consecutive lines (optionally paired
— Python's 2-line frames) are checked against a classifier; runs of
≥ N are collapsed to a placeholder with a count.

```yaml
pattern:
  kind: frame-run
  classifier: starts_with  # starts_with | contains
  values: ["at org.springframework.", "at org.apache.tomcat."]
  unit: 1                  # 1 for Java; 2 for Python/Go; N for language quirks
transform:
  kind: collapse-run
  min_run: 3
  replacement: "[{n} framework frames omitted]"
```

### 5.3 `template-dedup` — repeated-line collapse

Lines whose *normalized* form matches (volatile fields like numbers,
hashes, UUIDs replaced by placeholders) are grouped into
`[×N] <exemplar>`. This catches Maven repeat, Cargo `Compiling`
lines, Docker layer pulls, retry loops.

```yaml
pattern:
  kind: template-dedup
  normalize:
    - regex: "\\b\\d+\\b"            # collapse all integers
      replacement: "§"
    - regex: "\\b[0-9a-f]{8,}\\b"    # hex
      replacement: "§"
transform:
  kind: dedup
  min_run: 2
  format: "[×{n}] {exemplar}"
```

### 5.4 `prefix-factor` — common-prefix extraction

If ≥ N % of consecutive lines share a leading prefix, extract it
once. Catches cargo warning spam, test-suite path echoes.

```yaml
pattern:
  kind: prefix-factor
  min_share: 0.80
  min_lines: 4
transform:
  kind: factor-prefix
  replacement: "[prefix: {prefix}]\n  {suffixes}"
```

## 6. Transforms

Four transforms cover every rule shipped today in `layer1_filter.rs`:

| Transform | Input | Output |
|---|---|---|
| `delete` | line | nothing |
| `rewrite` | line, captures | one rewritten line |
| `collapse-run` | run of N matching lines | one summary line |
| `dedup` | N equivalent templates | one exemplar + `[×N]` |
| `factor-prefix` | N lines sharing prefix | one prefix header + N stripped suffixes |

Transforms are **pure functions** of their input. No
environmental reads, no filesystem access, no network.

## 7. Severity

Each rule declares one of three severities:

- `lossy-safe` — no user-visible information is lost. Example:
  collapsing 50 identical retry warnings into `[×50]`.
- `lossy-risky` — information is lost but the rule was authored
  with a trade-off in mind. Example: dropping ALL `INFO` lines from
  a container log. Clients default to skipping `lossy-risky` rules
  and must opt in.
- `info` — rule is a transformation that does not reduce content
  (e.g. timestamp normalization). Runs regardless of aggression.

## 8. Intent scope (the dimension ESLint lacked)

A 300-line stack trace during `"run my tests"` is noise. During
`"debug this failing test"` it is the entire point. Rules can
optionally declare which intents they *should* or *should not*
fire under:

```yaml
intent_scope:
  preserve_on: [debug_stack, fix_failing_test]
  apply_on:    [run_tests, ci_check, build_project]
```

The client supplies a classified intent label (see §10) via the
same mechanism NTK's Layer 4 already uses — the agent's most recent
user message reduced to a known label. If no classification is
available, rules fire as if `apply_on` matched — so the spec
degrades gracefully to "no intent awareness" on clients that don't
implement §10.

The published intent vocabulary (v1):

| Intent | Trigger phrase examples |
|---|---|
| `run_tests` | "run the tests", "check if it passes" |
| `fix_failing_test` | "this test is failing, fix it" |
| `debug_stack` | "why is this crashing", "trace this error" |
| `build_project` | "build it", "compile" |
| `ci_check` | "run ci", "verify lint+test" |
| `explore_codebase` | "how does X work", "show me Y" |
| `generic` | fallback |

Clients classify via string-match on the user message or (advanced)
a tiny local classifier. The vocabulary is intentionally small in
v1; future RFC extends it.

## 9. Invariants (the contract with the LLM)

Rules declare which invariants they respect. The runtime can
**reject** a rule that would violate an invariant it claims to
respect — this is what makes community-contributed rules safe to
accept.

v1 invariants (match NTK's current L1/L2 guarantees):

1. `preserve_errors` — lines containing `error`, `ERROR`, `panic:`,
   `Caused by`, `Traceback`, `Exception`, `E0[0-9]{3}:`, `FAILED`,
   `fatal`, `warning:` always survive.
2. `preserve_first_frame` — in a stack trace, the first frame
   (where user code entered framework land) stays.
3. `preserve_last_frame` — the final frame (entrypoint) stays.
4. `preserve_error_message` — the line(s) *before* a stack trace
   that name the exception/panic survive.
5. `deterministic` — same input ⇒ same output, byte-identical.
6. `idempotent` — `f(f(x)) == f(x)`.
7. `no_new_errors` — the rule must not produce output containing
   the marker patterns from (1) unless they were already present.

The runtime checks (1)–(4) post-hoc with a regex scan of input vs
output. (5)–(7) are tested offline by the runtime's test suite.

## 10. Cross-platform handling

Three real-world platform quirks the spec must address.

### 10.1 Path separators

Rules use `/` in classifier values. The runtime normalizes the
input stream before pattern match:

- Windows paths like `C:\Users\foo\site-packages\django\` are
  normalized to `C:/Users/foo/site-packages/django/` for matching
  only — the output preserves the original backslashes.
- Mixed-quoting (PowerShell often prints `'C:\...'`) is handled by
  stripping surrounding quotes before matching.

### 10.2 Line endings

Input is split on `\r\n | \r | \n`. Output uses `\n` uniformly.
Rules never see a line containing `\r`.

### 10.3 Command-name canonicalization

Category detection maps command variants to one category:

- `cargo`, `cargo +nightly`, `rustc` → `build_log` for Rust
- `npm`, `pnpm`, `yarn`, `bun run test` → `test_output` when `test`
  appears in argv, else `build_log`
- `pwsh`, `powershell`, `cmd` → the shell is only informational;
  the category is decided by what the command *inside* the shell
  produced.

## 11. Versioning and compatibility

The spec uses **semver with a twist**:

- `spec_version: 0.X` — pre-1.0, breaking changes allowed on each
  bump with migration guide
- `spec_version: 1.X` — additive changes only (new rule kinds,
  new invariants, new severities); existing rule files continue to
  parse
- `spec_version: 2.X` — next breaking epoch

A rule file that declares `spec_version: 0.1` must continue to
parse on a runtime that advertises `0.1` even after the runtime
adds support for `0.2` — the runtime picks up new keys only if
`spec_version` matches or is higher.

## 12. Rule-file directory convention

Ecosystem rules live under a standard layout:

```
ctxlint-rules/
├── spec_version
├── stack_trace/
│   ├── python.yaml
│   ├── java.yaml
│   ├── go.yaml
│   └── ...
├── test_output/
│   ├── cargo.yaml
│   ├── pytest.yaml
│   └── ...
├── build_log/
│   ├── cargo.yaml
│   ├── webpack.yaml
│   └── ...
├── container_log/
│   ├── docker.yaml
│   └── kubectl.yaml
├── diff/
│   └── git.yaml
└── generic/
    └── ansi_strip.yaml
```

The repo is distributed as:

- **crates.io** — `ctxlint-rules = "0.1"` with embedded YAML assets
- **npm** — `@ctxlint/rules` for Node/TS consumers
- **git submodule** — for anyone pinning to a specific commit
- **CDN snapshot** — JSON index at `https://ctxlint.org/v0.1/rules.json`
  for MCP servers that can't ship files

## 13. Reference implementation contract

A conforming implementation must:

- **Parse** `spec_version: 0.1` YAML/JSON rule files without error
- **Dispatch** lines to the four transforms in section 5/6
- **Enforce** invariants 1–7 from section 9 by post-hoc check
- **Honor** intent_scope when an intent is supplied, degrade to
  "apply" when it isn't
- **Report** applied rules per compression (for observability —
  NTK's `applied_rules` field is the model)
- **Cache-safe** — identical input + identical ruleset version =
  identical output (a cache key for result memoization)

NTK's Rust L1 ships as the reference implementation. Second-binding
work (issue #26) validates that another language/runtime (likely
Continue's TS extension or a Python package) can produce the same
output from the same rule files.

## 14. Prior art considered

- **ESLint / Stylelint** — shared rule format + plugin ecosystem,
  but ignores the *intent* axis because lint runs statically. We
  take the format lesson and reject the static assumption.
- **Detekt / rubocop** — language-specific. Successful because they
  accept that style rules don't cross languages. Stack-trace
  patterns *do* cross languages (framework collapse is universal),
  which justifies a shared spec.
- **OpenTelemetry semantic conventions** — shared attribute names
  across languages for telemetry. Same shape as what we propose:
  ecosystem-shared names, language-specific implementations.
- **strftime** — spec that froze a small format language and
  survived 40 years. Target shape: small, conservative, boring.

## 15. Open questions (to be resolved before v1.0)

1. **Transform composition** — can rules chain? `collapse-run`
   then `template-dedup` on the result. v1 says no (each line
   passes through at most one transform); that may be too
   restrictive. Benchmark with real fixtures before deciding.

2. **Per-rule cost estimate** — should rules declare a rough
   `cost: "O(n)" | "O(n²)"` so the runtime can reject quadratic
   regexes on large inputs? Useful but risks becoming folklore.

3. **Reverse mapping** — given compressed output, can we generate
   a "click here to see the original" pointer? Useful for debug UX
   but doubles storage. Probably a v2 concern.

4. **Negotiated intent vocabulary** — who maintains the list in
   section 8? Likely the ruleset repo itself (each release pins
   the vocabulary); clients support the intersection of their
   runtime's vocab and the file's `spec_version`.

5. **Security — rule-file sourcing** — a malicious rule could
   `delete` error lines and poison the model context. The runtime
   MUST enforce invariants 1–4 regardless of what a rule claims
   (the `invariants` field is a contract, not a toggle). Before
   v1.0 we need a "rule-file signing" story — probably
   `minisign`/`cosign` over tarballs.

## 16. Migration path from NTK's current code

Step 1 (already done by NTK): all current L1 rules are hand-coded
Rust and pass the invariant tests.

Step 2 (issue #24, POC Etapa 1): port **one language** — Python —
from hardcoded Rust to a YAML file loaded at runtime. Compare
output byte-for-byte; measure overhead. **Kill criterion:** > 20 %
overhead vs hardcoded means the abstraction is wrong, not just
slow.

Step 3 (issue #25, POC Etapa 2): publish this RFC, open 30-day
comment window. Gate: ≥ 100 stars OR ≥ 5 external contributors.

Step 4 (issue #26, POC Etapa 3): port the ruleset to a second
runtime (Continue TS plugin or a Python package). Produce identical
output from identical rules. **Kill criterion:** if the second
runtime needs rule-file modifications, the schema is wrong — v2
epoch.

## 17. Appendix — full example rule files

### 17.1 Python stack-trace ruleset (derived from current NTK)

```yaml
spec_version: 0.1
category: stack_trace
language: python
frameworks: [django, flask, fastapi, gunicorn, werkzeug, asgiref]
rules:
  - id: py.stack.site_packages
    applies_to: stack_trace.frame_run
    pattern:
      kind: frame-run
      classifier: contains
      values: ["/site-packages/"]
      unit: 2   # File "...", line N, in fn  (+ indented body line)
    transform:
      kind: collapse-run
      min_run: 3
      replacement: "[{n} framework frames omitted]"
    severity: lossy-safe
    invariants: [preserve_first_frame, preserve_last_frame, preserve_errors]

  - id: py.stack.django_core
    applies_to: stack_trace.frame_run
    pattern:
      kind: frame-run
      classifier: contains
      values: ["/django/core/", "/django/db/", "/django/http/"]
      unit: 2
    transform:
      kind: collapse-run
      min_run: 3
      replacement: "[{n} Django framework frames omitted]"
    severity: lossy-safe
    invariants: [preserve_first_frame, preserve_last_frame, preserve_errors]

  - id: py.stack.asgi_runtime
    applies_to: stack_trace.frame_run
    pattern:
      kind: frame-run
      classifier: contains
      values: ["/gunicorn/", "/asgiref/", "/werkzeug/", "/uvicorn/"]
      unit: 2
    transform:
      kind: collapse-run
      min_run: 3
      replacement: "[{n} ASGI/WSGI runtime frames omitted]"
    severity: lossy-safe
    invariants: [preserve_first_frame, preserve_last_frame, preserve_errors]
```

### 17.2 Cross-platform Docker log ruleset

```yaml
spec_version: 0.1
category: container_log
tool: docker
rules:
  - id: docker.layer_pull_dedup
    applies_to: container_log
    pattern:
      kind: template-dedup
      normalize:
        - regex: "\\b[0-9a-f]{12,}\\b"   # layer shas
          replacement: "§"
    transform:
      kind: dedup
      min_run: 2
      format: "[×{n}] {exemplar}"
    severity: lossy-safe
    invariants: [preserve_errors]

  - id: docker.pulling_progress
    applies_to: container_log
    pattern:
      kind: line-match
      classifier: regex
      values: ["^Pulling fs layer$", "^Waiting$", "^Pull complete$"]
    transform:
      kind: delete
    severity: lossy-safe
    invariants: [preserve_errors]
```

### 17.3 Windows-specific command-output rules

```yaml
spec_version: 0.1
category: generic
platform: windows
rules:
  - id: win.powershell.write_progress
    applies_to: generic
    pattern:
      kind: line-match
      classifier: regex
      values: ["^\\s*\\[oO]*\\]\\s*\\d+\\s*%"]
    transform:
      kind: delete
    severity: lossy-safe

  - id: win.cmd.xcopy_per_file
    applies_to: generic
    pattern:
      kind: template-dedup
      normalize:
        - regex: ".+\\\\.+"    # any path
          replacement: "§"
    transform:
      kind: dedup
      min_run: 5
      format: "[×{n}] {exemplar}"
    severity: lossy-safe
    invariants: [preserve_errors]
```

## 18. What "acceptance" means

This RFC becomes **v0.1-accepted** when:

- At least 3 non-maintainer comments with substantive technical
  input have been posted on issue #25
- The reference implementation (NTK Rust) ships the Python POC
  from §16 step 2 passing all invariant tests
- A second runtime (per §16 step 4) produces byte-identical output
  from the same rule files on the same fixtures

Until then, the schema may change. After v0.1-accepted, semver rules
in section 11 apply.

---

**Document authored:** 2026-04-19 · **Open for comment when
traction gate (issue #23) is met.**
