// Layer 1 — fast deterministic filter (no ML, no allocations-per-line beyond
// what's necessary). Runs on every /compress call, regardless of output size.
//
// Pipeline stages, in order:
//   1.  Strip ANSI escape codes.
//   2.  Detect RTK pre-filtered output (short-circuit flag only).
//   3.  Remove progress bars / spinner lines.
//   4.  Normalize volatile fields (timestamps, UUIDs, hex IDs, numbers) into
//       placeholders — *without* modifying the emitted output, only to build
//       template signatures for step 5.
//   5.  Template-aware repeated-line grouping: any two lines whose normalized
//       form is identical are collapsed to "[×N] <representative>".
//   6.  Stack-trace boilerplate filter: Spring/Tomcat/Django/Rails/Node/
//       CGLIB/Rust panic machinery collapsed to "... N framework frames".
//   7.  Common-prefix factoring: if ≥ 80 % of lines share a leading prefix,
//       extract it once.
//   8.  Test failure extraction (only when text looks like cargo test / vitest).
//   9.  Collapse consecutive blank lines.
//
// Design goals:
//   - Never lose the first / last stack frame of a trace.
//   - Preserve at least one exemplar of every deduplicated template.
//   - Deterministic (same input -> same output byte-for-byte).
//   - Safe for all languages: uses regex patterns only for universally
//     unambiguous tokens (timestamps, hex of length ≥ 8, UUIDs, plain ints).

use once_cell::sync::Lazy;
use regex::Regex;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct Layer1Result {
    pub output: String,
    pub rtk_pre_filtered: bool,
    pub lines_removed: usize,
    /// Human-readable list of pipeline stages that actually modified the
    /// input. Populated in-order, e.g. `["ansi_strip(142 chars)",
    /// "progress_removed(4 lines)", "template_dedup(3 groups)"]`. Safe to
    /// ignore; only consumed by `ntk test-compress --verbose`.
    pub applied_rules: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn filter(input: &str) -> Layer1Result {
    // Security: reject inputs that are unreasonably large before processing.
    // Callers are expected to enforce max_input_chars from config, but this
    // is a hard backstop so Layer 1 never allocates unboundedly.
    let input = if input.len() > 10_000_000 {
        &input[..10_000_000]
    } else {
        input
    };

    let mut applied_rules: Vec<String> = Vec::new();

    let stripped = strip_ansi(input);
    if stripped.len() < input.len() {
        let chars = input.len().saturating_sub(stripped.len());
        applied_rules.push(format!("ansi_strip({chars} chars)"));
    }
    let rtk_pre_filtered = detect_rtk_output(&stripped);
    if rtk_pre_filtered {
        applied_rules.push("rtk_pre_filtered".to_string());
    }

    let original_line_count = stripped.lines().count();

    let after_progress = remove_progress_bars(&stripped);
    let progress_removed = stripped
        .lines()
        .count()
        .saturating_sub(after_progress.lines().count());
    if progress_removed > 0 {
        applied_rules.push(format!("progress_removed({progress_removed} lines)"));
    }

    let after_diag = remove_diagnostic_noise(&after_progress);
    let diag_removed = after_progress
        .lines()
        .count()
        .saturating_sub(after_diag.lines().count());
    if diag_removed > 0 {
        applied_rules.push(format!("diagnostic_noise({diag_removed} lines)"));
    }

    let after_template_dedup = group_by_template(&after_diag);
    let dedup_groups = after_template_dedup
        .matches("[×")
        .count()
        .saturating_sub(after_progress.matches("[×").count());
    if dedup_groups > 0 {
        applied_rules.push(format!("template_dedup({dedup_groups} groups)"));
    }

    let after_stack_filter = filter_stack_frames(&after_template_dedup);
    let stack_runs = after_stack_filter.matches("frames omitted").count();
    if stack_runs > 0 {
        applied_rules.push(format!("stack_trace_collapse({stack_runs} runs)"));
    }

    // filter_test_failures must run BEFORE factor_common_prefix: it drops the
    // bulk of `test foo ... ok` lines when the suite reports FAILED, and the
    // factor stage would otherwise turn that bulk into `↳` rows that survive
    // the test-failure extractor (which keys on the original line shape).
    let after_test = filter_test_failures(&after_stack_filter);
    if after_test.lines().count() < after_stack_filter.lines().count() {
        let removed = after_stack_filter
            .lines()
            .count()
            .saturating_sub(after_test.lines().count());
        applied_rules.push(format!("test_failure_extracted({removed} lines)"));
    }

    // collapse_blank_lines runs BEFORE block_dedup and factor_common_prefix
    // so those stages see a regularized blank-line structure on every pass.
    // Without this, pass 1 sees uneven blanks (no block match), pass 2 sees
    // regular blanks (block fires) and idempotency is broken.
    let after_blanks = collapse_blank_lines(&after_test);
    let blanks_collapsed = after_test
        .lines()
        .count()
        .saturating_sub(after_blanks.lines().count());
    if blanks_collapsed > 0 {
        applied_rules.push(format!("blank_lines_collapsed({blanks_collapsed})"));
    }

    let after_blocks = collapse_repeated_blocks(&after_blanks);
    let block_markers = after_blocks.matches(BLOCK_MARKER_PREFIX).count();
    if block_markers > 0 {
        applied_rules.push(format!("block_dedup({block_markers} runs)"));
    }

    // Suffix factor BEFORE prefix factor: suffix catches longer trailing
    // strings (e.g. dotnet warning rule + message ~150 chars) which beats
    // the shared file-path prefix on most fixtures. Trade-off: docker-style
    // service-prefix outputs lose some compression (per-service prefix can
    // no longer cluster after the ↰ markers absorb the bulk). Net across
    // the corpus is strongly positive (+95pp vs. prefix-first ordering).
    let after_suffix = factor_common_suffix(&after_blocks);
    if after_suffix != after_blocks {
        applied_rules.push("common_suffix_factored".to_string());
    }

    let output = factor_common_prefix(&after_suffix);
    if output != after_suffix {
        applied_rules.push("common_prefix_factored".to_string());
    }

    let final_line_count = output.lines().count();
    let lines_removed = original_line_count.saturating_sub(final_line_count);

    Layer1Result {
        output,
        rtk_pre_filtered,
        lines_removed,
        applied_rules,
    }
}

// ---------------------------------------------------------------------------
// Step 1 — Strip ANSI escape codes
// ---------------------------------------------------------------------------

fn strip_ansi(input: &str) -> String {
    let bytes = strip_ansi_escapes::strip(input);
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Step 2 — Detect RTK pre-filtered output
// ---------------------------------------------------------------------------

fn detect_rtk_output(input: &str) -> bool {
    input.contains("[×")
}

// ---------------------------------------------------------------------------
// Step 3 — Remove progress bars / spinner lines
// ---------------------------------------------------------------------------

static PROGRESS_PATTERNS: &[&str] = &[
    "[====",
    "[####",
    "[----",
    "⠋",
    "⠙",
    "⠹",
    "⠸",
    "⠼",
    "⠴",
    "⠦",
    "⠧",
    "⠇",
    "⠏",
    "Downloading",
    " Downloading ",
];

// Cargo/rustc status markers. `contains` is too lax for these (a log
// line that quotes cargo output would be eaten), so they're matched
// against the trimmed prefix only. Cargo always emits them with a
// right-aligned verb followed by a space.
//
// These carry no signal on success — the final `Finished` line gives
// the verdict, and on failure the actual error follows below with the
// crate name included anyway.
static CARGO_PROGRESS_PREFIXES: &[&str] = &[
    "Compiling ",
    "Checking ",
    "Building ",
    "Fresh ",
    "Installing ",
    "Updating ",
];

fn remove_progress_bars(input: &str) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(input.lines().count());

    for line in input.lines() {
        let trimmed = line.trim_start_matches('\r');
        if trimmed.is_empty() && line.contains('\r') {
            continue;
        }
        if PROGRESS_PATTERNS.iter().any(|pat| trimmed.contains(pat)) {
            continue;
        }
        if is_cargo_progress(trimmed) {
            continue;
        }
        out.push(line);
    }

    out.join("\n")
}

fn is_cargo_progress(line: &str) -> bool {
    // Cargo right-aligns verbs to column 12 with a 3-4 space left pad.
    // Require the pad so a user-authored `Compiling X` line without
    // indentation never matches. Also require a version-or-path tail
    // (cargo always adds one) so a bare "   Compiling this manually"
    // comment-like line stays.
    let bytes = line.as_bytes();
    if bytes.len() < 4 || bytes[0] != b' ' {
        return false;
    }
    let trimmed = line.trim_start();
    if !CARGO_PROGRESS_PREFIXES
        .iter()
        .any(|p| trimmed.starts_with(p))
    {
        return false;
    }
    // `xxx v1.2.3` or `xxx (path)` — both forms have either `v<digit>`
    // or `(` after the word. Cheap substring check; the leading pad +
    // verb check already locked this to real cargo output.
    trimmed.contains(" v") || trimmed.contains(" (")
}

// ---------------------------------------------------------------------------
// Step 3.5 — Drop universally-noisy diagnostic lines
//
// Two narrow targets, both common across language toolchains:
//
//   1. TypeScript / Rust / Elm "wavy underline" rows — pure tilde +
//      whitespace + caret characters. They highlight the previous
//      content line, which already carries the same span as a
//      `^^^^` annotation; the tilde row is redundant for an LLM.
//
//   2. Git porcelain noise per file: `index abc..def 100644`,
//      `--- a/path`, `+++ b/path`. The `diff --git a/x b/x` header
//      one line above already names the file. The `index` SHAs are
//      volatile (rebase / amend invalidates them) and useless to
//      the model.
//
// Both deletions are line-conservative: lines that contain *any*
// content beyond the noise pattern stay. An error_signal line will
// never start with `~` or `index `, so the preserve_errors invariant
// holds trivially.
// ---------------------------------------------------------------------------

fn remove_diagnostic_noise(input: &str) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(input.lines().count());
    for line in input.lines() {
        if is_wavy_underline(line) || is_git_diff_metadata(line) {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

fn is_wavy_underline(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Accept only lines composed of `~`, `^`, and whitespace.
    trimmed
        .chars()
        .all(|c| c == '~' || c == '^' || c.is_whitespace())
        // Require at least 3 tildes/carets so a stray `~` in code
        // (e.g. JS bitwise NOT in a snippet) doesn't get caught.
        && trimmed.chars().filter(|c| *c == '~' || *c == '^').count() >= 3
}

fn is_git_diff_metadata(line: &str) -> bool {
    // `index abc1234..def5678 100644` — SHA range + file mode.
    // Match liberally on the prefix; the tail varies (line ranges,
    // missing mode for new files, etc).
    if let Some(rest) = line.strip_prefix("index ") {
        let head: String = rest.chars().take(20).collect();
        if head.contains("..") {
            return true;
        }
    }
    // `--- a/path/to/file` and `+++ b/path/to/file` per-file diff
    // markers. The `diff --git a/x b/x` line one above already
    // names both sides.
    if let Some(rest) = line.strip_prefix("--- ") {
        if rest.starts_with("a/") || rest == "/dev/null" {
            return true;
        }
    }
    if let Some(rest) = line.strip_prefix("+++ ") {
        if rest.starts_with("b/") || rest == "/dev/null" {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Step 4 + 5 — Template normalization and template-aware dedup
//
// Builds a "template signature" for each line by replacing volatile tokens:
//   ISO-8601 timestamps      → <TS>
//   UUIDs (8-4-4-4-12 hex)   → <UUID>
//   Hex of length ≥ 8        → <HEX>
//   Base64-ish ≥ 16          → <B64>
//   Plain integers ≥ 2 digs  → <N>   (e.g. 200, 5ms, 1024)
//
// Lines with the same template are considered equivalent for dedup purposes.
// The output preserves the first-seen exemplar with the count prefix.
// ---------------------------------------------------------------------------

// Static regexes are compiled from string literals at first use; the
// `.expect(..)` below can only panic if the literal pattern is invalid,
// which is caught by tests in CI. Annotating each one individually lets
// the security-gate clippy lints stay enabled elsewhere.
#[allow(clippy::expect_used)]
static RE_TS_ISO: Lazy<Regex> = Lazy::new(|| {
    // 2026-04-15T10:23:45Z, 2026-04-15T10:23:45.123456+00:00, 2026-04-15 10:23:45
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?")
        .expect("ISO timestamp regex must compile")
});

#[allow(clippy::expect_used)]
static RE_UUID: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
        .expect("UUID regex must compile")
});

#[allow(clippy::expect_used)]
static RE_LONG_HEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{8,}\b").expect("hex regex must compile"));

#[allow(clippy::expect_used)]
static RE_PLAIN_INT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(^|[^\w.])(\d+)").expect("integer regex must compile"));

// Kubernetes-style replica suffix: appears immediately after a long hex hash
// that LONG_HEX has already collapsed. e.g. `api-server-7d9c4b8f6c-2x8jk`
// becomes `api-server-<HEX>-2x8jk` after LONG_HEX, and this rule turns the
// trailing 5-char alphanumeric pod hash into `<POD>`. Anchored to `<HEX>-`
// so it cannot fire on arbitrary 5-char words like "hello".
#[allow(clippy::expect_used)]
static RE_REPLICA_POD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<HEX>-[a-z0-9]{5}\b").expect("replica pod regex must compile"));

// Package + semver inline: `react 18.2.0`, `@types/node 20.11.30`,
// `Compiling foo v0.1.0`. Replaces both the identifier AND the version with
// placeholders so `+ react 18.2.0` and `+ vue 3.4.21` collapse to the same
// template `+ <PKG> <VER>`. The leading boundary char is preserved.
#[allow(clippy::expect_used)]
static RE_PKG_VER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?P<pre>(?:^|[\s+(\[]))(?P<pkg>@?[a-zA-Z][\w@/.-]*?)\s+v?\d+\.\d+\.\d+(?:\.\d+)?(?:-[\w.]+)?",
    )
    .expect("package-version regex must compile")
});

// Bare semver (no preceding identifier): `Building 1.4.2`, `release v0.2.33`.
// Runs after PKG_VER so the package-aware rule wins when both could match.
#[allow(clippy::expect_used)]
static RE_VER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\bv?\d+\.\d+\.\d+(?:\.\d+)?(?:-[\w.]+)?\b").expect("version regex must compile")
});

// Gradle task header: `> Task :module:subproject:taskName` with optional
// trailing status like ` UP-TO-DATE`, ` SKIPPED`, ` FROM-CACHE`. Different
// modules/tasks should collapse into one template so consecutive task lines
// dedup. Anchored at line start so it cannot fire mid-message.
#[allow(clippy::expect_used)]
static RE_GRADLE_TASK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(> Task :)\S+(?: [A-Z][A-Z\-]+)?$").expect("gradle task regex must compile")
});

fn normalize_to_template(line: &str) -> String {
    // Order matters: timestamps must run before long-hex (they share digits);
    // LONG_HEX must run before REPLICA_POD (which keys on the <HEX> placeholder);
    // PKG_VER must run before VER and PLAIN_INT so the contextual match wins.
    let step1 = RE_TS_ISO.replace_all(line, "<TS>");
    let step2 = RE_UUID.replace_all(&step1, "<UUID>");
    let step3 = RE_LONG_HEX.replace_all(&step2, "<HEX>");
    let step4 = RE_REPLICA_POD.replace_all(&step3, "<HEX>-<POD>");
    let step5 = RE_PKG_VER.replace_all(&step4, "$pre<PKG> <VER>");
    let step6 = RE_VER.replace_all(&step5, "<VER>");
    let step7 = RE_PLAIN_INT.replace_all(&step6, "${1}<N>");
    let step8 = RE_GRADLE_TASK.replace_all(&step7, "${1}<TASK>");
    step8.into_owned()
}

fn group_by_template(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut idx = 0usize;

    while idx < lines.len() {
        let current = lines[idx];

        // Blank / whitespace-only lines are handled by the dedicated blank-line
        // collapse stage later. Grouping them here would produce marker lines
        // like "[×2] " that violate invariant #2 (exemplar must be readable).
        if current.trim().is_empty() {
            out.push(current.to_owned());
            idx = idx.saturating_add(1);
            continue;
        }

        // Idempotency: rows emitted by any later compression stage must
        // pass through untouched. Otherwise the digit/path placeholders
        // inside the marker become normalize-targets and re-running the
        // pipeline would over-collapse them — invariant #5.
        if is_already_processed_marker(current) {
            out.push(current.to_owned());
            idx = idx.saturating_add(1);
            continue;
        }

        let current_template = normalize_to_template(current);
        let mut count = 1usize;

        // Scan forward while subsequent lines share the same template.
        while idx.saturating_add(count) < lines.len() {
            let next = lines[idx.saturating_add(count)];
            if next.trim().is_empty() {
                break;
            }
            let next_template = normalize_to_template(next);
            if next_template != current_template {
                break;
            }
            count = count.saturating_add(1);
        }

        if count > 1 {
            out.push(format!("[×{count}] {current}"));
        } else {
            out.push(current.to_owned());
        }

        idx = idx.saturating_add(count);
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// Step 6 — Stack-trace boilerplate filter (multi-language)
//
// Recognizes consecutive frames that belong to framework / runtime
// machinery and collapses them. The classifier is conservative: a line
// must match a known framework pattern AND be part of a run of ≥ 3 such
// lines to be replaced. This protects user code and the first frame of
// unknown traces.
// ---------------------------------------------------------------------------

fn is_framework_frame(line: &str) -> bool {
    let t = line.trim_start();

    // --- JVM / Java ---
    if t.starts_with("at org.springframework.")
        || t.starts_with("at org.apache.catalina.")
        || t.starts_with("at org.apache.tomcat.")
        || t.starts_with("at org.apache.coyote.")
        || t.starts_with("at javax.servlet.")
        || t.starts_with("at jakarta.servlet.")
        || t.starts_with("at jdk.internal.reflect.")
        || t.starts_with("at java.lang.reflect.")
        || t.contains("$$FastClassBySpringCGLIB$$")
        || t.contains("$$EnhancerBySpringCGLIB$$")
        || t.contains("CglibAopProxy")
        || t.contains("ReflectiveMethodInvocation")
    {
        return true;
    }

    // --- Python / Django ---
    if t.starts_with("File \"")
        && (t.contains("/django/")
            || t.contains("/site-packages/")
            || t.contains("\\site-packages\\")
            || t.contains("/gunicorn/")
            || t.contains("/werkzeug/")
            || t.contains("/wsgiref/")
            || t.contains("/asgiref/"))
    {
        return true;
    }

    // --- Ruby / Rails ---
    if (t.contains("actionpack")
        || t.contains("activesupport")
        || t.contains("activerecord")
        || t.contains("actionview")
        || t.contains("railties")
        || t.contains("/rack/"))
        && (t.starts_with("from ") || t.contains(".rb:"))
    {
        return true;
    }

    // --- Node.js / Express ---
    if t.starts_with("at ")
        && (t.contains("node:internal/")
            || t.contains("node_modules/express/")
            || t.contains("node_modules\\express\\")
            || t.contains("Layer.handle ")
            || t.contains("next (")
            || t.contains("Function.handle ")
            || t.contains("Function.dispatch "))
    {
        return true;
    }

    // --- Go runtime ---
    // main.main is the user entrypoint — never filter.
    if t.starts_with("main.main(") {
        return false;
    }
    if t.starts_with("runtime.") {
        return true;
    }
    // Go frame body lines that live under /usr/local/go/src/runtime/ or
    // in the $GOROOT runtime dir are also framework.
    if t.starts_with('/') && (t.contains("/go/src/runtime/") || t.contains("\\go\\src\\runtime\\"))
    {
        return true;
    }

    // --- PHP / Symfony / Laravel ---
    // Classic "#N /path/..." trace frames.
    if t.starts_with("#")
        && (t.contains("/vendor/symfony/")
            || t.contains("/vendor/laravel/")
            || t.contains("/vendor/illuminate/")
            || t.contains("\\vendor\\symfony\\")
            || t.contains("\\vendor\\laravel\\"))
    {
        return true;
    }
    // Class-prefix frames that PHP emits in "at Namespace\Class->method()"
    // format (no path, no leading #). Symfony and Laravel's core vendor
    // frames collapse safely.
    if t.starts_with("at Symfony\\")
        || t.starts_with("at Illuminate\\")
        || t.starts_with("at Laravel\\")
    {
        return true;
    }
    // Inline-path continuation lines under an "at" frame:
    // "     (/app/vendor/symfony/event-dispatcher/...)" — carry no user
    // signal, they just echo the vendor path of the preceding "at" line.
    if t.starts_with('(')
        && (t.contains("/vendor/symfony/")
            || t.contains("/vendor/laravel/")
            || t.contains("/vendor/illuminate/"))
    {
        return true;
    }

    // --- Rust panic machinery ---
    if t.contains("core::panicking::")
        || t.contains("std::panicking::")
        || t.contains("rust_panic")
        || t.contains("rust_begin_unwind")
    {
        return true;
    }

    // --- .NET / C# / ASP.NET Core ---
    // Frames look like: "   at Microsoft.AspNetCore.Foo.Bar(...)"
    //                   "   at System.Threading.Tasks.Task.Xyz()"
    if t.starts_with("at Microsoft.AspNetCore.")
        || t.starts_with("at Microsoft.Extensions.")
        || t.starts_with("at System.Threading.Tasks.")
        || t.starts_with("at System.Runtime.ExceptionServices.")
        || t.starts_with("at System.Runtime.CompilerServices.")
        || t.starts_with("at Microsoft.EntityFrameworkCore.")
    {
        return true;
    }

    // --- JavaScript / TypeScript (browser bundlers + zone.js) ---
    if t.starts_with("at ")
        && (t.contains("webpack://")
            || t.contains("webpack-internal:")
            || t.contains("node_modules/react-dom/")
            || t.contains("node_modules\\react-dom\\")
            || t.contains("node_modules/react/")
            || t.contains("node_modules\\react\\")
            || t.contains("node_modules/next/")
            || t.contains("node_modules\\next\\")
            || t.contains("__zone_symbol__")
            || t.contains("zone.js"))
    {
        return true;
    }

    // --- React Native / Metro ---
    if t.starts_with("at ")
        && (t.contains("node_modules/react-native/")
            || t.contains("node_modules\\react-native\\")
            || t.contains("node_modules/metro-")
            || t.contains("node_modules\\metro-")
            || t.contains("ReactNativeRenderer"))
    {
        return true;
    }

    // --- Kotlin / Android ---
    if t.starts_with("at androidx.")
        || t.starts_with("at com.android.")
        || t.starts_with("at kotlinx.coroutines.")
        || t.starts_with("at kotlin.coroutines.")
        || t.starts_with("at dalvik.system.")
    {
        return true;
    }

    false
}

/// Returns true if the given line looks like the *body* of a Python-style
/// stack frame (the indented source line that follows a `File "..."` entry).
/// These lines don't themselves match any framework pattern, but when they
/// follow a framework `File "..."` they belong to a framework frame-group.
fn is_python_frame_body(line: &str) -> bool {
    let trimmed = line.trim_start();
    // Heuristic: the body is indented (4+ spaces) and not a new File/from/at
    // entry. Tight enough to not swallow user code that happens to appear
    // between frames.
    line.starts_with("    ")
        && !trimmed.is_empty()
        && !trimmed.starts_with("File \"")
        && !trimmed.starts_with("at ")
        && !trimmed.starts_with("from ")
}

/// Returns true if `line` looks like the source-location line that follows a
/// Go stack frame (typically "\t\t/path/to/file.go:LINE").
fn is_go_frame_body(line: &str) -> bool {
    let trimmed = line.trim_start();
    line.starts_with('\t') && trimmed.contains(".go:")
}

/// Extended classifier: treats a Python framework `File "..."` followed by its
/// code body as a single unit, so runs are counted by *frame* not by line.
fn is_framework_frame_unit(lines: &[&str], idx: usize) -> (bool, usize) {
    if idx >= lines.len() {
        return (false, 0);
    }
    if is_framework_frame(lines[idx]) {
        // Python: `File "..."` framework line followed by indented body.
        let next = idx.saturating_add(1);
        if lines[idx].trim_start().starts_with("File \"")
            && next < lines.len()
            && is_python_frame_body(lines[next])
        {
            return (true, 2);
        }
        // Go: `runtime.foo()` followed by `\t\t/path/file.go:LINE`.
        if lines[idx].trim_start().starts_with("runtime.")
            && next < lines.len()
            && is_go_frame_body(lines[next])
        {
            return (true, 2);
        }
        return (true, 1);
    }
    (false, 1)
}

fn filter_stack_frames(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < 3 {
        return input.to_owned();
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;

    while i < lines.len() {
        let (is_fw, unit_len) = is_framework_frame_unit(&lines, i);
        if !is_fw {
            out.push(lines[i].to_owned());
            i = i.saturating_add(unit_len.max(1));
            continue;
        }

        // Count the run of consecutive framework *units* starting at i.
        let mut cursor = i;
        let mut units = 0usize;
        while cursor < lines.len() {
            let (fw, len) = is_framework_frame_unit(&lines, cursor);
            if !fw {
                break;
            }
            cursor = cursor.saturating_add(len);
            units = units.saturating_add(1);
        }

        if units >= 3 {
            // Keep the first framework unit verbatim so the user sees where
            // the trace enters framework territory.
            let first_len = is_framework_frame_unit(&lines, i).1;
            for j in 0..first_len {
                if let Some(line) = lines.get(i.saturating_add(j)) {
                    out.push((*line).to_owned());
                }
            }
            let collapsed = units.saturating_sub(1);
            let indent = leading_whitespace(lines[i]);
            out.push(format!("{indent}... {collapsed} framework frames omitted"));
        } else {
            // Short run — keep verbatim.
            for line in lines.iter().take(cursor).skip(i) {
                out.push((*line).to_owned());
            }
        }

        i = cursor;
    }

    out.join("\n")
}

fn leading_whitespace(line: &str) -> String {
    line.chars().take_while(|c| c.is_whitespace()).collect()
}

// ---------------------------------------------------------------------------
// Step 7 — Common-prefix factoring
//
// If ≥ 80 % of non-blank lines share a leading prefix of ≥ 8 characters,
// emit the prefix once at the top and strip it from each line.
// Only fires when the saving actually beats the overhead (prefix length ×
// line count > prefix_line + "↳ ..." marker cost).
// ---------------------------------------------------------------------------

fn longest_common_prefix(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let first = lines[0];
    let mut end = first.len();
    for other in lines.iter().skip(1) {
        let shared = first
            .char_indices()
            .zip(other.char_indices())
            .take_while(|((_, a), (_, b))| a == b)
            .last()
            .map(|((i, c), _)| i.saturating_add(c.len_utf8()))
            .unwrap_or(0);
        end = end.min(shared);
        if end == 0 {
            break;
        }
    }
    first[..end].to_owned()
}

const PREFIX_HEADER_MARKER: &str = "── common prefix:";
const PREFIX_SUFFIX_MARKER: &str = "↳ ";
const SUFFIX_HEADER_MARKER: &str = "── common suffix:";
const SUFFIX_LEFT_MARKER: &str = "↰ ";

fn longest_common_suffix(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let first_chars: Vec<char> = lines[0].chars().collect();
    let mut max_chars = first_chars.len();

    for other in lines.iter().skip(1) {
        let other_chars: Vec<char> = other.chars().collect();
        let min_len = max_chars.min(other_chars.len());
        let mut match_chars = 0usize;
        for i in 1..=min_len {
            let f = first_chars[first_chars.len().saturating_sub(i)];
            let o = other_chars[other_chars.len().saturating_sub(i)];
            if f == o {
                match_chars = i;
            } else {
                break;
            }
        }
        max_chars = max_chars.min(match_chars);
        if max_chars == 0 {
            break;
        }
    }
    if max_chars == 0 {
        return String::new();
    }
    let start = lines[0]
        .char_indices()
        .rev()
        .take(max_chars)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    lines[0][start..].to_owned()
}

/// Mirror of `factor_common_prefix`. Discovers shared trailing substrings
/// (≥ `MIN_PREFIX_LEN` chars, ≥ `MIN_CLUSTER_LINES` rows) and emits:
///
///   ── common suffix: <suffix> ──
///   ↰ <prefix-only>
///   ↰ <prefix-only>
///
/// Targets dotnet/eslint-style outputs where every row ends with the same
/// rule + message but starts with a unique file path. Runs BEFORE
/// `factor_common_prefix` so the residual `↰ <prefix-only>` rows can also
/// have their common leading file-path factored.
fn factor_common_suffix(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let non_blank: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();

    if non_blank.len() < MIN_CLUSTER_LINES {
        return input.to_owned();
    }

    let processable: Vec<&str> = non_blank
        .iter()
        .copied()
        .filter(|l| !is_already_processed_marker(l))
        .collect();
    if processable.len() < MIN_CLUSTER_LINES {
        return input.to_owned();
    }

    // Sort by reversed string so common-suffix lines cluster contiguously.
    let mut sorted = processable.clone();
    sorted.sort_unstable_by(|a, b| {
        let a_rev: String = a.chars().rev().collect();
        let b_rev: String = b.chars().rev().collect();
        a_rev.cmp(&b_rev)
    });

    let mut clusters: Vec<String> = Vec::new();
    let mut claimed: Vec<bool> = vec![false; sorted.len()];

    while clusters.len() < MAX_PREFIX_CLUSTERS {
        let unclaimed: Vec<usize> = (0..sorted.len()).filter(|i| !claimed[*i]).collect();
        if unclaimed.len() < MIN_CLUSTER_LINES {
            break;
        }

        let mut best_suffix = String::new();
        let mut best_savings: usize = 0;

        let max_start = unclaimed.len().saturating_sub(MIN_CLUSTER_LINES);
        for ws in 0..=max_start {
            let window: Vec<&str> = unclaimed[ws..ws.saturating_add(MIN_CLUSTER_LINES)]
                .iter()
                .map(|&i| sorted[i])
                .collect();
            let lcs = longest_common_suffix(&window);
            if lcs.len() < MIN_PREFIX_LEN {
                continue;
            }

            let mut end = ws.saturating_add(MIN_CLUSTER_LINES);
            while end < unclaimed.len() && sorted[unclaimed[end]].ends_with(&lcs) {
                end = end.saturating_add(1);
            }
            let count = end.saturating_sub(ws);
            let savings = lcs.len().saturating_mul(count);
            let header_cost = lcs.len().saturating_add(PREFIX_HEADER_OVERHEAD);
            if savings <= header_cost {
                continue;
            }
            if savings > best_savings {
                best_savings = savings;
                best_suffix = lcs;
            }
        }

        if best_suffix.is_empty() {
            break;
        }

        for &i in &unclaimed {
            if sorted[i].ends_with(&best_suffix) {
                claimed[i] = true;
            }
        }
        clusters.push(best_suffix);
    }

    if clusters.is_empty() {
        return input.to_owned();
    }

    clusters.sort_by_key(|s| std::cmp::Reverse(s.len()));

    let mut header_emitted: Vec<bool> = vec![false; clusters.len()];
    let mut out: Vec<String> = Vec::with_capacity(lines.len().saturating_add(clusters.len()));

    for line in &lines {
        if is_already_processed_marker(line) {
            out.push((*line).to_owned());
            continue;
        }
        let mut handled = false;
        for (idx, suffix) in clusters.iter().enumerate() {
            if line.ends_with(suffix.as_str()) {
                if !header_emitted[idx] {
                    out.push(format!("{SUFFIX_HEADER_MARKER} {suffix} ──"));
                    header_emitted[idx] = true;
                }
                let cut = line.len().saturating_sub(suffix.len());
                let prefix_part = &line[..cut];
                out.push(format!("{SUFFIX_LEFT_MARKER}{prefix_part}"));
                handled = true;
                break;
            }
        }
        if !handled {
            out.push((*line).to_owned());
        }
    }

    out.join("\n")
}
// Block-dedup marker uses `──` framing (NOT `[×`) so the proptest
// `prop_dedup_keeps_real_exemplar` doesn't expect an exemplar body after the
// marker — the preceding emitted block IS the exemplar.
const BLOCK_MARKER_PREFIX: &str = "── ×";

/// N-gram block deduplication. Detects runs of ≥ BLOCK_MIN_RUN consecutive
/// blocks of identical normalized templates and replaces all but the first
/// with a `[×K more identical N-line block(s) omitted]` marker.
///
/// Targets multi-line repeats that single-line `group_by_template` cannot
/// handle: terraform `+ resource` blocks, repeated maven plugin invocations
/// per module, repeated bazel compile/link pairs. Runs after the per-line
/// stages so each line is already in its smallest representative form.
/// Lines emitted by an earlier compression stage (template dedup, prefix
/// factor, block dedup itself, stack-trace collapse) must never be folded
/// into a new block — they have already abstracted the underlying repeat,
/// and re-collapsing them violates idempotency (invariant #5).
fn is_already_processed_marker(line: &str) -> bool {
    line.starts_with(PREFIX_HEADER_MARKER)
        || line.starts_with(PREFIX_SUFFIX_MARKER)
        || line.starts_with(SUFFIX_HEADER_MARKER)
        || line.starts_with(SUFFIX_LEFT_MARKER)
        || line.starts_with(BLOCK_MARKER_PREFIX)
        || line.starts_with("[×")
        || line.contains("frames omitted]")
}

fn collapse_repeated_blocks(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < BLOCK_MIN_SIZE.saturating_mul(BLOCK_MIN_RUN) {
        return input.to_owned();
    }

    // Pre-compute normalized templates; same shape ↔ block-equivalent.
    let templates: Vec<String> = lines.iter().map(|l| normalize_to_template(l)).collect();
    let processed: Vec<bool> = lines
        .iter()
        .map(|l| is_already_processed_marker(l))
        .collect();

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;

    while i < lines.len() {
        // Markers from earlier stages are barriers: never start a block here
        // and never include them in block content.
        if processed[i] {
            out.push(lines[i].to_owned());
            i = i.saturating_add(1);
            continue;
        }

        let mut best_n = 0usize;
        let mut best_count = 0usize;
        let mut best_savings = 0usize;

        let remaining = lines.len().saturating_sub(i);
        let max_n = remaining.min(BLOCK_MAX_SIZE);

        for n in BLOCK_MIN_SIZE..=max_n {
            // Disallow any candidate block that contains a barrier marker.
            if (0..n).any(|k| processed[i.saturating_add(k)]) {
                break;
            }
            // Count consecutive identical blocks of size n starting at i.
            let mut count = 1usize;
            loop {
                let next_start = i.saturating_add(n.saturating_mul(count));
                let next_end = next_start.saturating_add(n);
                if next_end > lines.len() {
                    break;
                }
                if templates[i..i.saturating_add(n)] == templates[next_start..next_end] {
                    count = count.saturating_add(1);
                } else {
                    break;
                }
            }
            if count >= BLOCK_MIN_RUN {
                // (count-1) blocks of n lines each are dropped; subtract 1
                // for the marker line we add.
                let savings = count.saturating_sub(1).saturating_mul(n).saturating_sub(1);
                if savings > best_savings {
                    best_savings = savings;
                    best_n = n;
                    best_count = count;
                }
            }
        }

        if best_n > 0 {
            for k in 0..best_n {
                out.push(lines[i.saturating_add(k)].to_owned());
            }
            let extra = best_count.saturating_sub(1);
            out.push(format!(
                "{BLOCK_MARKER_PREFIX}{extra} more {best_n}-line block(s) omitted ──"
            ));
            i = i.saturating_add(best_n.saturating_mul(best_count));
        } else {
            out.push(lines[i].to_owned());
            i = i.saturating_add(1);
        }
    }

    out.join("\n")
}

fn factor_common_prefix(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let non_blank: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();

    if non_blank.len() < MIN_CLUSTER_LINES {
        return input.to_owned();
    }

    // Idempotency: ignore lines that look already-factored when discovering
    // clusters (and pass them through untouched in the emit phase).
    let processable: Vec<&str> = non_blank
        .iter()
        .copied()
        .filter(|l| !is_already_processed_marker(l))
        .collect();
    if processable.len() < MIN_CLUSTER_LINES {
        return input.to_owned();
    }

    // Greedy multi-cluster discovery: lex-sorting groups same-prefix lines
    // into contiguous runs, so for a window of MIN_CLUSTER_LINES sorted lines
    // the LCP is the longest prefix that ≥ that many lines share. We extend
    // each candidate cluster to its full lex-adjacent run, score by total
    // savings, pick the best, mark its lines, and repeat. This subsumes the
    // older single-cluster behaviour while also catching mixed inputs (e.g.
    // a Next.js route table with `/api/*` and `/blog/*` clusters at once).
    let mut sorted = processable.clone();
    sorted.sort_unstable();

    let mut clusters: Vec<String> = Vec::new();
    let mut claimed: Vec<bool> = vec![false; sorted.len()];

    while clusters.len() < MAX_PREFIX_CLUSTERS {
        let unclaimed: Vec<usize> = (0..sorted.len()).filter(|i| !claimed[*i]).collect();
        if unclaimed.len() < MIN_CLUSTER_LINES {
            break;
        }

        let mut best_prefix = String::new();
        let mut best_savings: usize = 0;

        let max_start = unclaimed.len().saturating_sub(MIN_CLUSTER_LINES);
        for ws in 0..=max_start {
            let window: Vec<&str> = unclaimed[ws..ws.saturating_add(MIN_CLUSTER_LINES)]
                .iter()
                .map(|&i| sorted[i])
                .collect();
            let lcp = longest_common_prefix(&window);
            if lcp.len() < MIN_PREFIX_LEN {
                continue;
            }

            // Extend the cluster across all lex-adjacent siblings that still
            // start with this prefix.
            let mut end = ws.saturating_add(MIN_CLUSTER_LINES);
            while end < unclaimed.len() && sorted[unclaimed[end]].starts_with(&lcp) {
                end = end.saturating_add(1);
            }
            let count = end.saturating_sub(ws);
            let savings = lcp.len().saturating_mul(count);
            let header_cost = lcp.len().saturating_add(PREFIX_HEADER_OVERHEAD);
            if savings <= header_cost {
                continue;
            }
            if savings > best_savings {
                best_savings = savings;
                best_prefix = lcp;
            }
        }

        if best_prefix.is_empty() {
            break;
        }

        for &i in &unclaimed {
            if sorted[i].starts_with(&best_prefix) {
                claimed[i] = true;
            }
        }
        clusters.push(best_prefix);
    }

    if clusters.is_empty() {
        return input.to_owned();
    }

    // Longer prefixes win when a line matches several (e.g. "/api/users/" vs
    // "/api/"), so the more-specific factoring shows up in the output.
    clusters.sort_by_key(|p| std::cmp::Reverse(p.len()));

    let mut header_emitted: Vec<bool> = vec![false; clusters.len()];
    let mut out: Vec<String> = Vec::with_capacity(lines.len().saturating_add(clusters.len()));

    for line in &lines {
        if is_already_processed_marker(line) {
            out.push((*line).to_owned());
            continue;
        }
        let mut handled = false;
        for (idx, prefix) in clusters.iter().enumerate() {
            if line.starts_with(prefix.as_str()) {
                if !header_emitted[idx] {
                    out.push(format!("{PREFIX_HEADER_MARKER} {prefix} ──"));
                    header_emitted[idx] = true;
                }
                out.push(format!("{PREFIX_SUFFIX_MARKER}{}", &line[prefix.len()..]));
                handled = true;
                break;
            }
        }
        if !handled {
            out.push((*line).to_owned());
        }
    }

    out.join("\n")
}

const MIN_CLUSTER_LINES: usize = 5;
const MIN_PREFIX_LEN: usize = 12;
const MAX_PREFIX_CLUSTERS: usize = 8;
const PREFIX_HEADER_OVERHEAD: usize = 22;
const BLOCK_MIN_SIZE: usize = 2;
const BLOCK_MAX_SIZE: usize = 50;
const BLOCK_MIN_RUN: usize = 3;

// ---------------------------------------------------------------------------
// Step 8 — Keep only failures in test output
// ---------------------------------------------------------------------------

fn is_test_output(input: &str) -> bool {
    input.contains("test result:") || input.contains("FAILED")
}

fn filter_test_failures(input: &str) -> String {
    if !is_test_output(input) {
        return input.to_owned();
    }

    let mut out: Vec<&str> = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        let is_passing_test_line = trimmed.starts_with("test ")
            && (trimmed.ends_with(" ... ok") || trimmed.ends_with(" ... ignored"));
        if !is_passing_test_line {
            out.push(line);
        }
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------
// Step 9 — Collapse consecutive blank lines to at most one
// ---------------------------------------------------------------------------

fn collapse_blank_lines(input: &str) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(input.lines().count());
    let mut prev_blank = false;

    for line in input.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue;
        }
        out.push(line);
        prev_blank = is_blank;
    }

    // Drop trailing blank lines so filter() is idempotent.
    // Without this, input "a\n\n\n\n" becomes "a\n" on the first pass (lines
    // ["a",""] joined by "\n") and then "a" on a second pass — a violation
    // of the prop_layer1_filter_is_idempotent proptest invariant.
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_suffix_factors_dotnet_warnings() {
        // Many lines with unique file paths but identical trailing rule +
        // message. Suffix factor should emit a single header + per-file
        // ↰ rows.
        let input = "\
src/a.cs(42,13): warning CS8618: Non-nullable property 'X' must contain a non-null value.
src/b.cs(42,13): warning CS8618: Non-nullable property 'X' must contain a non-null value.
src/c.cs(42,13): warning CS8618: Non-nullable property 'X' must contain a non-null value.
src/d.cs(42,13): warning CS8618: Non-nullable property 'X' must contain a non-null value.
src/e.cs(42,13): warning CS8618: Non-nullable property 'X' must contain a non-null value.
";
        let result = filter(input);
        // Group_by_template will collapse first since all 5 have same template.
        // For pure suffix-factor verification, use a fixture where templates differ.
        // Here templates DO match (paths get normalized). Either dedup or
        // suffix-factor is fine — both compress.
        assert!(
            result.output.contains("[×5]") || result.output.contains(SUFFIX_HEADER_MARKER),
            "expected dedup OR suffix factor: {}",
            result.output
        );
    }

    #[test]
    fn test_common_suffix_factors_distinct_template_lines() {
        // Different templates (different rule numbers, line numbers) but
        // shared trailing message. Template dedup CANNOT collapse; suffix
        // factor should.
        let input = "\
src/a.cs(11,1): warning CS1591: Missing XML comment for publicly visible type or member.
src/b.cs(22,2): warning CS1592: Missing XML comment for publicly visible type or member.
src/c.cs(33,3): warning CS1593: Missing XML comment for publicly visible type or member.
src/d.cs(44,4): warning CS1594: Missing XML comment for publicly visible type or member.
src/e.cs(55,5): warning CS1595: Missing XML comment for publicly visible type or member.
src/f.cs(66,6): warning CS1596: Missing XML comment for publicly visible type or member.
";
        let result = filter(input);
        assert!(
            result.output.contains(SUFFIX_HEADER_MARKER),
            "expected common-suffix factoring: {}",
            result.output
        );
        assert!(
            result.output.contains(SUFFIX_LEFT_MARKER),
            "expected ↰ rows: {}",
            result.output
        );
    }

    #[test]
    fn test_common_suffix_idempotent_on_factored_output() {
        let input = "\
file_a.txt :: tail-shared-content here
file_b.txt :: tail-shared-content here
file_c.txt :: tail-shared-content here
file_d.txt :: tail-shared-content here
file_e.txt :: tail-shared-content here
file_f.txt :: tail-shared-content here
";
        let once = filter(input).output;
        let twice = filter(&once).output;
        assert_eq!(once, twice, "factor_common_suffix must be idempotent");
    }

    #[test]
    fn test_common_suffix_does_not_fire_on_unique_endings() {
        let input = "\
alpha
beta
gamma
delta
epsilon
";
        let result = filter(input);
        assert!(
            !result.output.contains(SUFFIX_HEADER_MARKER),
            "suffix factor should not fire on unique endings: {}",
            result.output
        );
    }

    #[test]
    fn test_block_dedup_collapses_repeated_terraform_resources() {
        // 4 identical 3-line blocks → 1 block + marker for the other 3.
        let block = "  + resource \"x\" {\n    + foo = \"bar\"\n    }";
        let input = format!("{block}\n{block}\n{block}\n{block}\n");
        let result = filter(&input);
        assert!(
            result.output.contains(BLOCK_MARKER_PREFIX),
            "expected block-dedup marker: {}",
            result.output
        );
        assert!(result.output.contains("3-line block(s) omitted"));
        // First block must survive intact.
        assert!(result.output.contains("+ resource"));
        assert!(result.output.contains("+ foo = \"bar\""));
    }

    #[test]
    fn test_block_dedup_idempotent_on_marker_output() {
        let block = "alpha\nbeta\ngamma";
        let input = format!("{block}\n{block}\n{block}\n{block}\n");
        let first = filter(&input).output;
        let second = filter(&first).output;
        assert_eq!(first, second, "block dedup must be idempotent");
    }

    #[test]
    fn test_block_dedup_does_not_fire_on_non_repeating_content() {
        let input = "\
unique line A
unique line B
unique line C
unique line D
unique line E
";
        let result = filter(input);
        assert!(
            !result.output.contains(BLOCK_MARKER_PREFIX),
            "block dedup must not fire on distinct lines: {}",
            result.output
        );
    }

    #[test]
    fn test_block_dedup_idempotent_with_uneven_blanks() {
        // Regression: pass 1 saw uneven blanks (2 then 1) and didn't match,
        // pass 2 saw collapse_blank_lines-regularized blanks and matched —
        // breaking idempotency. Putting collapse_blank_lines BEFORE block
        // dedup fixed this.
        let input = "\ntest module::test_fn ... ok\n\n\ntest module::test_fn ... ok\n\ntest module::test_fn ... ok\n";
        let once = filter(input).output;
        let twice = filter(&once).output;
        assert_eq!(once, twice, "not idempotent on uneven-blanks input");
    }

    #[test]
    fn test_normalize_gradle_task_dedups_runs() {
        let input = "\
> Task :core:compileKotlin
> Task :core:compileJava
> Task :data:compileKotlin
> Task :data:processResources
> Task :app:assemble UP-TO-DATE
";
        let result = filter(input);
        assert!(
            result.output.contains("[×5]"),
            "expected 5-line dedup of `> Task :…`: {}",
            result.output
        );
    }

    #[test]
    fn test_normalize_gradle_task_does_not_eat_unrelated_lines() {
        // Lines starting with `> ` but not `> Task :` should be untouched.
        let input = "\
> some other prompt
> not a gradle task
> also not gradle
";
        let result = filter(input);
        assert!(!result.output.contains("<TASK>"), "got: {}", result.output);
    }

    #[test]
    fn test_normalize_pkg_ver_collapses_npm_install() {
        // pnpm/npm install produces lines like "+ react 18.2.0". Different
        // packages should now share the same template so template-dedup
        // collapses the run. (No leading spaces — the `\<newline>` Rust
        // continuation strips them and would split the first line into a
        // distinct template.)
        let input = "\
+ react 18.2.0
+ react-dom 18.2.0
+ @types/node 20.11.30
+ typescript 5.4.3
+ vite 5.2.6
";
        let result = filter(input);
        assert!(
            result.output.contains("[×5]"),
            "expected 5-line dedup of `+ <pkg> <ver>`: {}",
            result.output
        );
    }

    #[test]
    fn test_normalize_ver_collapses_bare_versions() {
        // Lines like "Building module 1.4.2" should normalize the trailing
        // version so a run of them dedups regardless of the digits.
        let input = "\
Building module 1.4.2
Building module 1.4.3
Building module 2.0.0
";
        let result = filter(input);
        assert!(
            result.output.contains("[×3]"),
            "expected 3-line dedup of `Building module <ver>`: {}",
            result.output
        );
    }

    #[test]
    fn test_normalize_replica_pod_dedups_kubernetes_pods() {
        // After LONG_HEX collapses the 10-char replicaset hash, the trailing
        // 5-char pod-specific hash must also normalize so different replicas
        // of the same deployment share a template.
        let input = "\
pod/api-server-7d9c4b8f6c-2x8jk Started container api
pod/api-server-7d9c4b8f6c-h7m4p Started container api
pod/api-server-7d9c4b8f6c-9xqz4 Started container api
";
        let result = filter(input);
        assert!(
            result.output.contains("[×3]"),
            "expected 3-line dedup across pod replicas: {}",
            result.output
        );
    }

    #[test]
    fn test_normalize_pkg_ver_does_not_eat_unrelated_words() {
        // Negative test: lines that look like prose with no version should be
        // untouched and stay distinct.
        let input = "\
hello world
foo bar baz
quick brown fox
";
        let result = filter(input);
        // No dedup, no placeholders leaking into output.
        assert!(!result.output.contains("[×"));
        assert!(!result.output.contains("<PKG>"));
        assert!(!result.output.contains("<VER>"));
    }

    #[test]
    fn test_remove_ansi_codes() {
        let input = "\x1b[32mhello\x1b[0m world";
        let result = filter(input);
        assert_eq!(result.output, "hello world");
        assert!(!result.rtk_pre_filtered);
    }

    #[test]
    fn test_applied_rules_records_ansi_and_dedup() {
        // Input mixes ANSI escapes + 3 identical lines → both stages should fire.
        let input = "\x1b[32mfoo\x1b[0m\nfoo\nfoo\nother";
        let result = filter(input);
        let joined = result.applied_rules.join(",");
        assert!(
            joined.contains("ansi_strip"),
            "expected ansi_strip in applied_rules: {joined}"
        );
        assert!(
            joined.contains("template_dedup"),
            "expected template_dedup in applied_rules: {joined}"
        );
    }

    #[test]
    fn test_applied_rules_empty_for_clean_input() {
        // No ANSI, no duplicates, no progress bars → no rules should fire.
        let input = "line one\nline two\nline three";
        let result = filter(input);
        assert!(
            result.applied_rules.is_empty(),
            "expected no rules applied, got {:?}",
            result.applied_rules
        );
    }

    #[test]
    fn test_group_repeated_lines() {
        let input = "cargo:warning=foo\ncargo:warning=foo\ncargo:warning=foo\nother";
        let result = filter(input);
        assert!(result.output.contains("[×3] cargo:warning=foo"));
        assert!(result.output.contains("other"));
    }

    #[test]
    fn test_keep_only_failures_cargo_test() {
        let input = "\
test module::test_a ... ok
test module::test_b ... FAILED
test module::test_c ... ok
test result: FAILED. 2 passed; 1 failed";

        let result = filter(input);
        assert!(!result.output.contains("test_a"));
        assert!(!result.output.contains("test_c"));
        assert!(result.output.contains("test_b"));
        assert!(result.output.contains("test result:"));
    }

    #[test]
    fn test_remove_progress_bars() {
        let input = "Downloading crate foo\nnormal line\n[==== 70%]\nanother line";
        let result = filter(input);
        assert!(!result.output.contains("[===="));
        assert!(result.output.contains("normal line"));
        assert!(result.output.contains("another line"));
    }

    #[test]
    fn test_collapse_blank_lines() {
        let input = "line1\n\n\n\nline2\n\nline3";
        let result = filter(input);
        assert!(!result.output.contains("\n\n\n"));
        assert!(result.output.contains("line1"));
        assert!(result.output.contains("line2"));
        assert!(result.output.contains("line3"));
    }

    #[test]
    fn test_detect_rtk_filtered_output() {
        let input = "[×47] cargo:warning=unused import\n[×12] note: ...";
        let result = filter(input);
        assert!(result.rtk_pre_filtered);
    }

    // ----- New tests for improvements ----------------------------------

    #[test]
    fn test_template_dedup_timestamps() {
        let input = "\
2026-04-15T10:23:00Z INFO [api] GET /health 200 12ms
2026-04-15T10:23:01Z INFO [api] GET /health 200 14ms
2026-04-15T10:23:02Z INFO [api] GET /health 200 11ms
2026-04-15T10:23:03Z INFO [api] GET /health 200 13ms
";
        let result = filter(input);
        assert!(
            result.output.contains("[×4]"),
            "expected [×4] marker, got: {}",
            result.output
        );
        // Only one representative line plus the marker
        let representative_count = result.output.matches("INFO [api]").count();
        assert_eq!(representative_count, 1);
    }

    #[test]
    fn test_template_dedup_uuids() {
        let input = "\
request 550e8400-e29b-41d4-a716-446655440000 completed
request 6ba7b810-9dad-11d1-80b4-00c04fd430c8 completed
request 6ba7b811-9dad-11d1-80b4-00c04fd430c8 completed
";
        let result = filter(input);
        assert!(result.output.contains("[×3]"));
    }

    #[test]
    fn test_stack_trace_java_filter() {
        let input = "\
Exception in thread \"main\" java.lang.RuntimeException: Database error
    at com.example.MyService.findUser(MyService.java:42)
    at org.springframework.cglib.proxy.MethodProxy.invoke(MethodProxy.java:218)
    at org.springframework.aop.framework.CglibAopProxy$CglibMethodInvocation.invokeJoinpoint(CglibAopProxy.java:783)
    at org.springframework.aop.framework.ReflectiveMethodInvocation.proceed(ReflectiveMethodInvocation.java:163)
    at org.springframework.transaction.interceptor.TransactionInterceptor.invoke(TransactionInterceptor.java:119)
    at jdk.internal.reflect.GeneratedMethodAccessor42.invoke(Unknown Source)
    at java.lang.reflect.Method.invoke(Method.java:566)
    at com.example.Main.main(Main.java:10)
";
        let result = filter(input);
        assert!(result.output.contains("framework frames omitted"));
        assert!(result.output.contains("MyService.findUser"));
        assert!(result.output.contains("Main.main"));
    }

    #[test]
    fn test_stack_trace_python_django_filter() {
        let input = "\
Traceback (most recent call last):
  File \"/app/views.py\", line 42, in get_user
    user = User.objects.get(id=user_id)
  File \"/usr/lib/python3/site-packages/django/db/models/manager.py\", line 85, in get
    return self.get_queryset().get(*args, **kwargs)
  File \"/usr/lib/python3/site-packages/django/db/models/query.py\", line 431, in get
    raise self.model.DoesNotExist
  File \"/usr/lib/python3/site-packages/django/core/handlers/exception.py\", line 47, in inner
    response = get_response(request)
User.DoesNotExist: User matching query does not exist.
";
        let result = filter(input);
        assert!(result.output.contains("framework frames omitted"));
        assert!(result.output.contains("/app/views.py"));
        assert!(result.output.contains("User.DoesNotExist"));
    }

    #[test]
    fn test_common_prefix_factoring() {
        // Mix of prefixes that aren't identical templates (so template-dedup
        // doesn't collapse them first), yet share a long leading substring.
        let input = "\
very long shared leading prefix here AAA
very long shared leading prefix here BBB
very long shared leading prefix here CCC
very long shared leading prefix here DDD
very long shared leading prefix here EEE
very long shared leading prefix here FFF
";
        let result = filter(input);
        assert!(
            result.output.contains("common prefix") || result.output.contains("↳"),
            "expected common-prefix factoring: {}",
            result.output
        );
    }

    #[test]
    fn test_common_prefix_multi_cluster() {
        // Two distinct clusters in the same input: route paths split between
        // /api/ and /blog/. The single-cluster algorithm could only factor
        // one of them; multi-cluster discovery should emit both headers.
        let mut lines: Vec<String> = Vec::new();
        for i in 1..=8 {
            lines.push(format!("/api/v1/resource_{i:03}/handler"));
        }
        for i in 1..=8 {
            lines.push(format!("/blog/posts/2026/article_{i:03}"));
        }
        let result = filter(&lines.join("\n"));
        let header_count = result.output.matches("common prefix").count();
        assert!(
            header_count >= 2,
            "expected ≥2 cluster headers, found {header_count}: {}",
            &result.output[..result.output.len().min(400)]
        );
        assert!(
            result.output.contains("/api/v1/") && result.output.contains("/blog/posts/"),
            "both cluster prefixes should appear in headers: {}",
            &result.output[..result.output.len().min(400)]
        );
    }

    #[test]
    fn test_common_prefix_idempotent_on_factored_output() {
        // Re-running the filter on already-factored output must not nest
        // headers or re-cluster the ↳ rows. Critical for invariant #5.
        let mut lines: Vec<String> = (1..=20)
            .map(|i| format!("warning: unused variable: `tmp_{i}` --> src/foo.rs:42:9"))
            .collect();
        lines.push("DONE".to_owned());
        let first = filter(&lines.join("\n")).output;
        let second = filter(&first).output;
        assert_eq!(
            first, second,
            "factor_common_prefix must be idempotent on its own output"
        );
    }

    #[test]
    fn test_factor_handles_outlier_trailing_line() {
        // Real-world case: 200 warnings sharing a long prefix plus a trailing
        // "DONE" sentinel line. The naive LCP across all lines collapses to
        // 0 chars, but ≥ 80% of lines still share the warning prefix, so the
        // factoring must still fire. Regression test for the "echo DONE"
        // case where L1 was registering zero savings via the hook.
        let mut lines: Vec<String> = (1..=200)
            .map(|i| format!("warning: unused variable: `tmp_{i}` --> src/foo.rs:42:9"))
            .collect();
        lines.push("DONE".to_owned());
        let input = lines.join("\n");
        let result = filter(&input);
        // Either prefix OR suffix factor must fire — the bulk shares both
        // a common prefix (`warning: unused variable: \`tmp_`) and a common
        // suffix (`\` --> src/foo.rs:42:9`). Suffix factor runs first and
        // typically wins on this fixture; either is acceptable.
        assert!(
            result.output.contains("common prefix") || result.output.contains("common suffix"),
            "factor stage should fire despite the DONE outlier: {}",
            &result.output[..result.output.len().min(300)]
        );
        assert!(
            result.output.contains("DONE"),
            "outlier line must survive unchanged"
        );
    }

    #[test]
    fn test_common_prefix_not_fired_below_threshold() {
        let input = "\
   Compiling foo
   Compiling bar
totally different line
   Compiling baz
   Compiling qux
";
        // 4 of 5 lines share prefix = 80%, so still fires.
        // Make it truly below threshold with 3/5 = 60%.
        let under = "\
   Compiling a
different
line
   Compiling b
   Compiling c
";
        let result = filter(under);
        assert!(
            !result.output.contains("common prefix"),
            "should not factor when < 80% share prefix: {}",
            result.output
        );
        let _ = input;
    }

    #[test]
    fn test_no_loss_of_error_lines_in_repetitive_log() {
        // Ensure error/warn lines survive the template dedup.
        let input = "\
2026-04-15T10:23:00Z INFO request 100 ok
2026-04-15T10:23:01Z INFO request 101 ok
2026-04-15T10:23:02Z INFO request 102 ok
2026-04-15T10:23:03Z ERROR request 103 failed: timeout
2026-04-15T10:23:04Z INFO request 104 ok
";
        let result = filter(input);
        assert!(
            result.output.contains("ERROR") && result.output.contains("timeout"),
            "error line must be preserved: {}",
            result.output
        );
    }

    #[test]
    fn test_multi_language_stack_filter_go() {
        let input = "\
panic: runtime error: invalid memory address
goroutine 1 [running]:
main.handleRequest(0x12345)
        /app/handler.go:42
runtime.goexit()
        /usr/local/go/src/runtime/asm_amd64.s:1571
runtime.main()
        /usr/local/go/src/runtime/proc.go:255
runtime.schedule()
        /usr/local/go/src/runtime/proc.go:3056
runtime.findrunnable()
        /usr/local/go/src/runtime/proc.go:2600
";
        let result = filter(input);
        assert!(
            result.output.contains("main.handleRequest"),
            "user frame must survive"
        );
        assert!(
            result.output.contains("framework frames omitted")
                || !result.output.contains("runtime.findrunnable")
        );
    }

    #[test]
    fn test_stack_trace_csharp_aspnet_filter() {
        let input = "\
System.InvalidOperationException: Sequence contains no elements
   at System.Linq.ThrowHelper.ThrowNoElementsException()
   at MyApp.Services.OrderService.GetLatest(Int32 userId) in /app/Services/OrderService.cs:line 42
   at MyApp.Controllers.OrdersController.GetLatest(Int32 userId) in /app/Controllers/OrdersController.cs:line 28
   at Microsoft.AspNetCore.Mvc.Infrastructure.ControllerActionInvoker.InvokeActionMethodAsync()
   at Microsoft.AspNetCore.Mvc.Infrastructure.ControllerActionInvoker.InvokeNextActionFilterAsync()
   at Microsoft.AspNetCore.Mvc.Infrastructure.ResourceInvoker.InvokeFilterPipelineAsync()
   at Microsoft.AspNetCore.Routing.EndpointMiddleware.Invoke(HttpContext context)
   at Microsoft.AspNetCore.Authorization.AuthorizationMiddleware.Invoke(HttpContext context)
   at Microsoft.AspNetCore.Diagnostics.DeveloperExceptionPageMiddleware.Invoke(HttpContext context)
   at System.Threading.Tasks.Task.<>c.<ThrowAsync>b__128_0(Object state)
   at System.Runtime.ExceptionServices.ExceptionDispatchInfo.Throw()
";
        let result = filter(input);
        // Invariant #1 — exception message never dropped.
        assert!(result.output.contains("InvalidOperationException"));
        // User frames survive.
        assert!(result.output.contains("OrderService.GetLatest"));
        assert!(result.output.contains("OrdersController.GetLatest"));
    }

    #[test]
    fn test_stack_trace_ts_react_filter() {
        let input = "\
TypeError: Cannot read properties of null (reading 'x')
    at useLayoutEffect (webpack-internal:///./src/components/Modal.tsx:42:23)
    at commitLayoutEffectOnFiber (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:23168:17)
    at commitRootImpl (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:26825:5)
    at commitRoot (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:26546:5)
    at performConcurrentWorkOnRoot (webpack-internal:///./node_modules/react-dom/cjs/react-dom.development.js:25655:7)
    at workLoop (webpack-internal:///./node_modules/scheduler/cjs/scheduler.development.js:266:34)
    at ZoneDelegate.invokeTask (webpack-internal:///./node_modules/zone.js/bundles/zone.umd.js:412:31)
    at Object.onInvokeTask (__zone_symbol__ZoneAwareError.js:2:12)
";
        let result = filter(input);
        assert!(result.output.contains("TypeError"));
        // User frame (Modal.tsx) must survive; react-dom and zone frames can be collapsed.
        assert!(result.output.contains("Modal.tsx"));
    }

    #[test]
    fn test_stack_trace_php_symfony_class_prefix_filter() {
        // Regression guard for #21: PHP/Symfony traces now emit frames in
        // two forms NTK must collapse — the classic "#N /vendor/..."
        // format AND Symfony's "at Namespace\Class->method()" format with
        // no path, plus the indented "(/vendor/...)" continuation lines.
        let input = "\
Symfony\\Component\\HttpKernel\\Exception\\NotFoundHttpException: No route
  at MyApp\\Controller\\UserController->show()
  at Symfony\\Component\\HttpKernel\\EventListener\\RouterListener->onKernelRequest()
     (/app/vendor/symfony/event-dispatcher/EventDispatcher.php:270)
  at Symfony\\Component\\EventDispatcher\\EventDispatcher->callListeners()
     (/app/vendor/symfony/http-kernel/HttpKernel.php:139)
  at Illuminate\\Foundation\\Http\\Kernel->sendRequestThroughRouter()
  at Laravel\\Lumen\\Application->dispatch()
";
        let result = filter(input);
        // Error header + user frame survive.
        assert!(result.output.contains("NotFoundHttpException"));
        assert!(
            result.output.contains("UserController->show"),
            "user frame lost: {}",
            result.output
        );
        // At least one framework-collapse marker fires.
        assert!(
            result.output.contains("framework frames omitted"),
            "expected collapse on Symfony+Illuminate+Laravel frames: {}",
            result.output
        );
    }

    #[test]
    fn test_stack_trace_kotlin_android_filter() {
        let input = "\
java.lang.IllegalStateException: View must be attached
	at com.example.myapp.ui.home.HomeFragment.onViewCreated(HomeFragment.kt:58)
	at androidx.fragment.app.Fragment.performViewCreated(Fragment.java:3089)
	at androidx.fragment.app.FragmentStateManager.createView(FragmentStateManager.java:548)
	at androidx.lifecycle.LiveData.considerNotify(LiveData.java:133)
	at androidx.lifecycle.LiveData.dispatchingValue(LiveData.java:151)
	at kotlinx.coroutines.DispatchedTask.run(DispatchedTask.kt:108)
	at kotlinx.coroutines.scheduling.CoroutineScheduler.runSafely(CoroutineScheduler.kt:584)
	at kotlinx.coroutines.scheduling.CoroutineScheduler$Worker.run(CoroutineScheduler.kt:684)
	at com.android.internal.os.ZygoteInit.main(ZygoteInit.java:930)
";
        let result = filter(input);
        assert!(result.output.contains("IllegalStateException"));
        // User frame must survive.
        assert!(result.output.contains("HomeFragment.onViewCreated"));
    }

    #[test]
    fn test_diagnostic_noise_drops_wavy_underlines() {
        let input = "\
src/foo.tsx:10:3 - error TS2300: Type 'string' is not assignable to type 'number'.

10   const result = fetchUser(id);
    ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Found 1 error in 1 file.
";
        let result = filter(input);
        assert!(
            !result.output.contains("~~~~~~~~~~~~~~~~~~"),
            "wavy underline must be dropped: {}",
            result.output
        );
        // Error line preserved (preserve_errors invariant).
        assert!(result.output.contains("error TS2300"));
        assert!(result.output.contains("Found 1 error"));
    }

    #[test]
    fn test_diagnostic_noise_drops_git_index_and_per_file_headers() {
        let input = "\
diff --git a/src/server.rs b/src/server.rs
index abc1234..def5678 100644
--- a/src/server.rs
+++ b/src/server.rs
@@ -45,12 +45,28 @@ pub struct CompressResponse {
     pub compressed: String,
+    pub tokens_after_l1: Option<usize>,
";
        let result = filter(input);
        assert!(
            !result.output.contains("index abc1234..def5678"),
            "git index line must be dropped: {}",
            result.output
        );
        assert!(
            !result.output.contains("--- a/src/server.rs"),
            "--- a/ marker must be dropped"
        );
        assert!(
            !result.output.contains("+++ b/src/server.rs"),
            "+++ b/ marker must be dropped"
        );
        // Hunk header + diff header MUST stay so the LLM still sees
        // which file changed and where.
        assert!(result.output.contains("diff --git a/src/server.rs"));
        assert!(result.output.contains("@@ -45"));
    }

    #[test]
    fn test_cargo_progress_lines_removed() {
        // Cargo right-aligns verbs with 3-4 leading spaces. We build
        // the input with explicit \n so the Rust string-continuation
        // escape doesn't eat our indentation.
        let input = [
            "   Compiling serde v1.0.0",
            "   Compiling tokio v1.35.0",
            "   Compiling axum v0.7.4",
            "    Checking ntk v0.2.28 (/home/user/ntk)",
            "    Building [===================>] 42/50",
            "    Finished `release` profile [optimized] target(s) in 1m 03s",
        ]
        .join("\n");
        let result = filter(&input);
        assert!(
            !result.output.contains("Compiling serde"),
            "cargo Compiling lines must be dropped: {}",
            result.output
        );
        assert!(
            !result.output.contains("Compiling tokio"),
            "cargo Compiling lines must be dropped"
        );
        assert!(
            !result.output.contains("Checking ntk"),
            "cargo Checking must be dropped"
        );
        // The final Finished line is informational and carries the
        // success verdict — we do NOT drop it.
        assert!(result.output.contains("Finished"));
    }

    #[test]
    fn test_cargo_progress_does_not_drop_unindented_compiling_in_prose() {
        // A user-authored log line that happens to start with
        // "Compiling" but has no indent MUST NOT be filtered.
        let input = "Compiling the shader took 3 seconds.\n";
        let result = filter(input);
        assert!(result.output.contains("Compiling the shader"));
    }

    #[test]
    fn test_diagnostic_noise_does_not_drop_user_code_with_tildes() {
        // A real code line containing a single `~` (JS bitwise NOT)
        // must NOT be classified as a wavy underline.
        let input = "\
const inverted = ~flags;
";
        let result = filter(input);
        assert!(result.output.contains("~flags"));
    }
}
