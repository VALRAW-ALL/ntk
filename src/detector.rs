// ---------------------------------------------------------------------------
// Output type detection
//
// Classifies the content of a Bash tool output into one of five categories
// so that Layer 2 and Layer 3 can apply type-specific compression strategies.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputType {
    /// cargo test, vitest, pytest, playwright — test runner output
    Test,
    /// cargo build/check, tsc, next build, eslint — compiler/linter output
    Build,
    /// docker logs, journalctl, nginx access log — structured log lines
    Log,
    /// git diff, git show, patch — unified diff format
    Diff,
    /// fallback when no specific pattern matches
    Generic,
}

// ---------------------------------------------------------------------------
// Detection logic (ordered by specificity / priority)
// ---------------------------------------------------------------------------

pub fn detect(input: &str) -> OutputType {
    if is_test(input) {
        OutputType::Test
    } else if is_build(input) {
        OutputType::Build
    } else if is_diff(input) {
        OutputType::Diff
    } else if is_log(input) {
        OutputType::Log
    } else {
        OutputType::Generic
    }
}

// ---------------------------------------------------------------------------
// Heuristics per type
// ---------------------------------------------------------------------------

fn is_test(input: &str) -> bool {
    // cargo test markers
    if input.contains("test result:") || input.contains("running 0 tests") {
        return true;
    }
    // FAILED line in test context — must also have "passed" or "failed" count
    if input.contains("FAILED") && (input.contains("passed") || input.contains("failed")) {
        return true;
    }
    // vitest
    if input.contains("✓") && input.contains("✗") {
        return true;
    }
    if input.contains("PASS") && input.contains("FAIL") && input.contains("Tests:") {
        return true;
    }
    // pytest
    if input.contains("passed") && input.contains("failed") && input.contains("===") {
        return true;
    }
    // playwright
    if input.contains("passed") && input.contains("failed") && input.contains("Playwright") {
        return true;
    }
    false
}

fn is_build(input: &str) -> bool {
    // Rust compiler errors
    if input.contains("error[E") {
        return true;
    }
    // TypeScript
    if input.contains("TS") && (input.contains("error TS") || input.contains("warning TS")) {
        return true;
    }
    // Rust warnings / cargo output
    if input.contains("warning[") || input.contains("Compiling ") || input.contains("Building ") {
        return true;
    }
    // ESLint
    if input.contains("eslint") || (input.contains("error") && input.contains("warning") && input.contains("problem")) {
        return true;
    }
    // Next.js build
    if input.contains("Creating an optimized production build") || input.contains("Route (app)") {
        return true;
    }
    false
}

fn is_diff(input: &str) -> bool {
    // Unified diff header
    if input.starts_with("diff --git") || input.starts_with("--- ") || input.starts_with("+++ ") {
        return true;
    }
    // Multiple hunk headers
    let hunk_count = input.lines().filter(|l| l.starts_with("@@ ")).count();
    if hunk_count >= 2 {
        return true;
    }
    false
}

fn is_log(input: &str) -> bool {
    // ISO timestamp pattern: lines starting with YYYY-MM-DD or YYYY/MM/DD
    let timestamp_lines = input
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            looks_like_timestamp(t)
        })
        .count();
    let total_lines = input.lines().count();
    // More than 30% of lines start with a timestamp → log output
    if total_lines > 3 && timestamp_lines.saturating_mul(10) > total_lines.saturating_mul(3) {
        return true;
    }
    // Structured log level prefixes
    let log_level_lines = input
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("[INFO]")
                || t.starts_with("[ERROR]")
                || t.starts_with("[WARN]")
                || t.starts_with("[DEBUG]")
                || t.starts_with("INFO ")
                || t.starts_with("ERROR ")
                || t.starts_with("WARN ")
                || t.starts_with("DEBUG ")
        })
        .count();
    if total_lines > 3 && log_level_lines.saturating_mul(10) > total_lines.saturating_mul(3) {
        return true;
    }
    false
}

/// Heuristic: does this string start with a timestamp-like prefix?
fn looks_like_timestamp(s: &str) -> bool {
    // YYYY-MM-DD or YYYY/MM/DD (ISO-style)
    let b = s.as_bytes();
    if b.len() < 10 {
        return false;
    }
    // bytes 0-3: digits; 4: '-' or '/'; 5-6: digits; 7: '-' or '/'; 8-9: digits
    b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && (b[4] == b'-' || b[4] == b'/')
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && (b[7] == b'-' || b[7] == b'/')
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_cargo_test_output() {
        let input = "\
test foo::test_a ... ok
test foo::test_b ... FAILED
test result: FAILED. 1 passed; 1 failed; 0 ignored";
        assert_eq!(detect(input), OutputType::Test);
    }

    #[test]
    fn test_detects_tsc_output() {
        let input = "src/index.ts(10,5): error TS2345: Argument of type 'string' is not assignable.";
        assert_eq!(detect(input), OutputType::Build);
    }

    #[test]
    fn test_detects_vitest_output() {
        let input = "\
 PASS  src/foo.test.ts
 FAIL  src/bar.test.ts

Tests:  1 failed, 5 passed, 6 total";
        assert_eq!(detect(input), OutputType::Test);
    }

    #[test]
    fn test_detects_docker_logs() {
        let input = "\
2024-03-15T10:00:00.000Z INFO server started on port 3000
2024-03-15T10:00:01.123Z WARN deprecated endpoint called
2024-03-15T10:00:02.456Z ERROR connection refused
2024-03-15T10:00:03.789Z INFO request processed";
        assert_eq!(detect(input), OutputType::Log);
    }

    #[test]
    fn test_detects_git_diff() {
        let input = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!(\"hello\");
 }";
        assert_eq!(detect(input), OutputType::Diff);
    }

    #[test]
    fn test_unknown_falls_back_to_generic() {
        let input = "some random output that matches no known pattern\nfoo bar baz";
        assert_eq!(detect(input), OutputType::Generic);
    }
}
