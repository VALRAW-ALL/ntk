// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct Layer1Result {
    pub output: String,
    pub rtk_pre_filtered: bool,
    pub lines_removed: usize,
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

    let stripped = strip_ansi(input);
    let rtk_pre_filtered = detect_rtk_output(&stripped);

    let original_line_count = stripped.lines().count();

    let after_progress = remove_progress_bars(&stripped);
    let after_dedup = group_repeated_lines(&after_progress);
    let after_test = filter_test_failures(&after_dedup);
    let output = collapse_blank_lines(&after_test);

    let final_line_count = output.lines().count();
    let lines_removed = original_line_count.saturating_sub(final_line_count);

    Layer1Result {
        output,
        rtk_pre_filtered,
        lines_removed,
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
//
// RTK-filtered output is already compact: no ANSI codes and contains the
// deduplication marker pattern "[×N]" (e.g. "[×47]").
// ---------------------------------------------------------------------------

fn detect_rtk_output(input: &str) -> bool {
    // Must not contain ANSI (already stripped) and must have [×N] markers.
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
    "⠏",             // spinner chars
    "Downloading",   // cargo download progress lines (repeated)
    " Downloading ", // with spaces for specificity
];

fn remove_progress_bars(input: &str) -> String {
    let mut out = Vec::with_capacity(input.lines().count());

    for line in input.lines() {
        // Lines that start with \r are overwrite-style progress bars.
        let trimmed = line.trim_start_matches('\r');

        // Skip blank overwrite lines.
        if trimmed.is_empty() && line.contains('\r') {
            continue;
        }

        // Skip lines that are pure progress bar content.
        let is_progress = PROGRESS_PATTERNS.iter().any(|pat| trimmed.contains(pat));

        if !is_progress {
            out.push(line);
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// Step 4 — Group repeated consecutive lines: "line [×N]"
// ---------------------------------------------------------------------------

fn group_repeated_lines(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut idx = 0usize;

    while idx < lines.len() {
        let current = lines[idx];
        let mut count = 1usize;

        while idx.saturating_add(count) < lines.len() && lines[idx.saturating_add(count)] == current
        {
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
// Step 5 — Keep only failures in test output
//
// If the text looks like a test runner output (contains "test result:" or
// "FAILED" marker), discard lines that record passing tests.
// Lines we remove: "test <name> ... ok"
// Lines we keep: failures, summary lines, stderr output, blank lines.
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
        // Drop lines that only record a passing test, e.g.:
        //   "test foo::bar_baz ... ok"
        //   "test foo::bar_baz ... ignored"
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
// Step 6 — Collapse consecutive blank lines to at most one
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
    fn test_group_repeated_lines() {
        let input = "cargo:warning=foo\ncargo:warning=foo\ncargo:warning=foo\nother";
        let result = filter(input);
        assert!(result.output.contains("[×3] cargo:warning=foo"));
        assert!(result.output.contains("other"));
        assert_eq!(result.lines_removed, 2);
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
        // At most one consecutive blank line
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
}
