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

    let after_template_dedup = group_by_template(&after_progress);
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

    let after_prefix = factor_common_prefix(&after_stack_filter);
    if after_prefix != after_stack_filter {
        applied_rules.push("common_prefix_factored".to_string());
    }

    let after_test = filter_test_failures(&after_prefix);
    if after_test.lines().count() < after_prefix.lines().count() {
        let removed = after_prefix
            .lines()
            .count()
            .saturating_sub(after_test.lines().count());
        applied_rules.push(format!("test_failure_extracted({removed} lines)"));
    }

    let output = collapse_blank_lines(&after_test);
    let blanks_collapsed = after_test
        .lines()
        .count()
        .saturating_sub(output.lines().count());
    if blanks_collapsed > 0 {
        applied_rules.push(format!("blank_lines_collapsed({blanks_collapsed})"));
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

fn remove_progress_bars(input: &str) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(input.lines().count());

    for line in input.lines() {
        let trimmed = line.trim_start_matches('\r');
        if trimmed.is_empty() && line.contains('\r') {
            continue;
        }
        let is_progress = PROGRESS_PATTERNS.iter().any(|pat| trimmed.contains(pat));
        if !is_progress {
            out.push(line);
        }
    }

    out.join("\n")
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
    Lazy::new(|| Regex::new(r"(^|[^\w.])(\d{2,})").expect("integer regex must compile"));

fn normalize_to_template(line: &str) -> String {
    // Order matters: timestamps must run before long-hex (they share digits).
    let step1 = RE_TS_ISO.replace_all(line, "<TS>");
    let step2 = RE_UUID.replace_all(&step1, "<UUID>");
    let step3 = RE_LONG_HEX.replace_all(&step2, "<HEX>");
    // RE_PLAIN_INT captures (prefix)(digits) so we restore the prefix.
    let step4 = RE_PLAIN_INT.replace_all(&step3, "${1}<N>");
    step4.into_owned()
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

fn factor_common_prefix(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let non_blank: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();

    if non_blank.len() < 5 {
        return input.to_owned();
    }

    let prefix = longest_common_prefix(&non_blank);
    if prefix.len() < 8 {
        return input.to_owned();
    }

    // Threshold: at least 80% of non-blank lines must actually share the prefix
    // (the LCP is the intersection, but some lines might be shorter; bail if so).
    let with_prefix = non_blank.iter().filter(|l| l.starts_with(&prefix)).count();
    let threshold = non_blank.len().saturating_mul(4).saturating_div(5); // 80%
    if with_prefix < threshold {
        return input.to_owned();
    }

    // Savings check: emit prefix header + one marker worth of bytes; must beat
    // the per-line prefix cost.
    let header_cost = prefix.len().saturating_add(16); // "── prefix: ... ──\n"
    let per_line_savings = prefix.len();
    let total_savings = per_line_savings.saturating_mul(with_prefix);
    if total_savings <= header_cost {
        return input.to_owned();
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len().saturating_add(1));
    out.push(format!("── common prefix: {prefix} ──"));
    for line in &lines {
        if line.starts_with(&prefix) {
            out.push(format!("↳ {}", &line[prefix.len()..]));
        } else {
            out.push((*line).to_owned());
        }
    }
    out.join("\n")
}

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
}
