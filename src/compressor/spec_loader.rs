// POC loader for RFC-0001 context-linter spec (#24).
//
// Scope of this MVP:
//   - Parses `spec_version: 0.1` rule files (YAML or JSON)
//   - Implements the ONE primitive that carries the most value:
//     `frame-run` with `classifier: contains` + `transform: collapse-run`
//   - Applies invariants #1/#3 (preserve_errors, preserve_first/last_frame)
//     as a post-hoc regex check — same contract as `layer1_filter.rs`
//
// What this does NOT cover yet (tracked in RFC-0001 §16 step 2):
//   - line-match, template-dedup, prefix-factor primitives
//   - rewrite / delete / factor-prefix transforms
//   - intent_scope gating
//   - severity ordering
//   - signing / trust chain
//
// The POC's job is to answer one question: **can an in-process YAML
// ruleset produce correct compression with acceptable overhead vs
// hardcoded Rust?** Everything else is polish once that yes/no is in.

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::path::Path;

// ---------------------------------------------------------------------------
// Schema — minimal subset of RFC-0001 §4.2
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RuleFile {
    pub spec_version: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    pub id: String,
    pub pattern: Pattern,
    pub transform: Transform,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub invariants: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Pattern {
    FrameRun {
        classifier: Classifier,
        values: Vec<String>,
        #[serde(default = "default_unit")]
        unit: usize,
    },
    LineMatch {
        classifier: Classifier,
        values: Vec<String>,
    },
    TemplateDedup {
        #[serde(default)]
        normalize: Vec<NormalizeRule>,
    },
    PrefixFactor {
        #[serde(default = "default_min_share")]
        min_share: f32,
        #[serde(default = "default_prefix_min_lines")]
        min_lines: usize,
    },
}

#[derive(Debug, Deserialize)]
pub struct NormalizeRule {
    pub regex: String,
    #[serde(default = "default_normalize_placeholder")]
    pub replacement: String,
}

fn default_normalize_placeholder() -> String {
    "§".to_string()
}

fn default_min_share() -> f32 {
    0.80
}

fn default_prefix_min_lines() -> usize {
    4
}

fn default_unit() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classifier {
    Contains,
    StartsWith,
    Regex,
    Equals,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Transform {
    CollapseRun {
        #[serde(default = "default_min_run")]
        min_run: usize,
        replacement: String,
    },
    Delete,
    Rewrite {
        replacement: String,
    },
    Dedup {
        #[serde(default = "default_dedup_min_run")]
        min_run: usize,
        #[serde(default = "default_dedup_format")]
        format: String,
    },
    FactorPrefix {
        #[serde(default = "default_prefix_replacement")]
        replacement: String,
    },
}

fn default_dedup_min_run() -> usize {
    2
}

fn default_dedup_format() -> String {
    "[×{n}] {exemplar}".to_string()
}

fn default_prefix_replacement() -> String {
    "[prefix: {prefix}]".to_string()
}

fn default_min_run() -> usize {
    3
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Load a rule file from disk. Accepts YAML or JSON (picks parser by
/// file extension; content-based detection is a future refinement).
pub fn load_rule_file(path: &Path) -> Result<RuleFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading rule file {}", path.display()))?;

    let is_json = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let file: RuleFile = if is_json {
        serde_json::from_str(&text).with_context(|| format!("parsing JSON {}", path.display()))?
    } else {
        serde_yaml::from_str(&text).with_context(|| format!("parsing YAML {}", path.display()))?
    };

    if !file.spec_version.starts_with("0.") {
        return Err(anyhow!(
            "unsupported spec_version '{}' — this loader handles 0.X",
            file.spec_version
        ));
    }

    Ok(file)
}

// ---------------------------------------------------------------------------
// Apply one rule file to input — the happy path for the POC
// ---------------------------------------------------------------------------

pub struct ApplyResult {
    pub output: String,
    /// IDs of rules whose pattern fired at least once.
    pub applied: Vec<String>,
    /// IDs of rules skipped because their invariants would have been
    /// violated by the transform (runtime refuses to ship a regression).
    pub invariant_rejected: Vec<String>,
}

/// Apply every rule in `file` to `input` in order. Later rules see
/// the output of earlier rules — same composition shape as the
/// hardcoded `filter_stack_frames` in layer1_filter.
pub fn apply_rule_file(input: &str, file: &RuleFile) -> ApplyResult {
    let mut current = input.to_owned();
    let mut applied = Vec::new();
    let mut rejected = Vec::new();

    for rule in &file.rules {
        let result = apply_one(&current, &rule.pattern, &rule.transform);
        let (new_text, fired) = match result {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("spec_loader: rule '{}' errored: {e}", rule.id);
                rejected.push(rule.id.clone());
                continue;
            }
        };

        if !fired {
            continue;
        }

        // Post-hoc invariant check — refuse to ship output that dropped
        // error signal even if the rule claimed to preserve it.
        if rule.invariants.iter().any(|i| i == "preserve_errors")
            && !preserves_error_signal(&current, &new_text)
        {
            rejected.push(rule.id.clone());
            continue;
        }

        current = new_text;
        applied.push(rule.id.clone());
    }

    ApplyResult {
        output: current,
        applied,
        invariant_rejected: rejected,
    }
}

/// Dispatch one (pattern, transform) pair to its implementation.
/// Returns (new_text, fired) — `fired=false` means the rule didn't
/// change anything and its id is not added to `applied`.
fn apply_one(input: &str, pattern: &Pattern, transform: &Transform) -> Result<(String, bool)> {
    match (pattern, transform) {
        (
            Pattern::FrameRun {
                classifier,
                values,
                unit,
            },
            Transform::CollapseRun {
                min_run,
                replacement,
            },
        ) => Ok(apply_frame_run(
            input,
            classifier,
            values,
            *unit,
            *min_run,
            replacement,
        )),

        (Pattern::LineMatch { classifier, values }, Transform::Delete) => {
            Ok(apply_line_match_delete(input, classifier, values))
        }

        (Pattern::LineMatch { classifier, values }, Transform::Rewrite { replacement }) => {
            apply_line_match_rewrite(input, classifier, values, replacement)
        }

        (Pattern::TemplateDedup { normalize }, Transform::Dedup { min_run, format }) => {
            apply_template_dedup(input, normalize, *min_run, format)
        }

        (
            Pattern::PrefixFactor {
                min_share,
                min_lines,
            },
            Transform::FactorPrefix { replacement },
        ) => Ok(apply_prefix_factor(
            input,
            *min_share,
            *min_lines,
            replacement,
        )),

        _ => Err(anyhow!(
            "unsupported (pattern, transform) combination in rule"
        )),
    }
}

// ---------------------------------------------------------------------------
// frame-run / collapse-run primitive
// ---------------------------------------------------------------------------

fn apply_frame_run(
    input: &str,
    classifier: &Classifier,
    values: &[String],
    unit: usize,
    min_run: usize,
    replacement_tpl: &str,
) -> (String, bool) {
    let unit = unit.max(1);
    let min_run = min_run.max(2);

    let lines: Vec<&str> = input.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    let mut any_fired = false;

    while i < lines.len() {
        // Check whether `unit` consecutive lines starting at i look like
        // a framework frame by the classifier.
        let is_frame = classifies_frame(&lines, i, unit, classifier, values);
        if !is_frame {
            out.push(lines[i].to_owned());
            i = i.saturating_add(1);
            continue;
        }

        // Greedy: collect as many consecutive framework frames as possible.
        let mut count = 0usize;
        while classifies_frame(
            &lines,
            i.saturating_add(count.saturating_mul(unit)),
            unit,
            classifier,
            values,
        ) {
            count = count.saturating_add(1);
        }

        if count >= min_run {
            // Collapse: emit the first frame verbatim, then the replacement
            // for the middle, then the last frame verbatim — matches
            // invariant #3 (preserve_first_frame + preserve_last_frame).
            // For `count == 2` this path doesn't run (min_run >= 2 check
            // above ensures count >= 3 at minimum when replacement fires).
            let first_start = i;
            let first_end = first_start.saturating_add(unit);
            for line in &lines[first_start..first_end.min(lines.len())] {
                out.push((*line).to_owned());
            }

            let omitted = count.saturating_sub(2); // minus first + last
            out.push(replacement_tpl.replace("{n}", &omitted.to_string()));

            let last_start = i.saturating_add(count.saturating_sub(1).saturating_mul(unit));
            let last_end = last_start.saturating_add(unit);
            for line in &lines[last_start..last_end.min(lines.len())] {
                out.push((*line).to_owned());
            }

            i = i.saturating_add(count.saturating_mul(unit));
            any_fired = true;
        } else {
            // Not enough consecutive frames to collapse — emit as-is.
            let end = i
                .saturating_add(count.saturating_mul(unit))
                .min(lines.len());
            for line in &lines[i..end] {
                out.push((*line).to_owned());
            }
            i = end;
        }
    }

    (out.join("\n"), any_fired)
}

fn classifies_frame(
    lines: &[&str],
    idx: usize,
    unit: usize,
    classifier: &Classifier,
    values: &[String],
) -> bool {
    if idx >= lines.len() {
        return false;
    }
    // For 2-line Python frames, the first line (File "...") carries the
    // signature; the indented body line below has no path. So we only
    // need offset 0 to match. The unit >= 2 bound check keeps the next
    // iteration from over-reading past the tail.
    if idx.saturating_add(unit.saturating_sub(1)) >= lines.len() {
        return false;
    }
    line_matches(lines[idx], classifier, values)
}

/// Shared predicate used by frame-run and line-match. Handles all four
/// classifier kinds; regex patterns compile once per call (callers are
/// expected to cache across invocations — this is a POC).
fn line_matches(line: &str, classifier: &Classifier, values: &[String]) -> bool {
    match classifier {
        Classifier::Contains => values.iter().any(|v| line.contains(v.as_str())),
        Classifier::StartsWith => {
            let t = line.trim_start();
            values.iter().any(|v| t.starts_with(v.as_str()))
        }
        Classifier::Equals => values.iter().any(|v| line == v),
        Classifier::Regex => values
            .iter()
            .filter_map(|v| Regex::new(v).ok())
            .any(|re| re.is_match(line)),
    }
}

// ---------------------------------------------------------------------------
// line-match primitive (delete + rewrite variants)
// ---------------------------------------------------------------------------

fn apply_line_match_delete(
    input: &str,
    classifier: &Classifier,
    values: &[String],
) -> (String, bool) {
    let mut out: Vec<&str> = Vec::with_capacity(input.lines().count());
    let mut fired = false;
    for line in input.lines() {
        if line_matches(line, classifier, values) {
            fired = true;
            continue;
        }
        out.push(line);
    }
    (out.join("\n"), fired)
}

fn apply_line_match_rewrite(
    input: &str,
    classifier: &Classifier,
    values: &[String],
    replacement: &str,
) -> Result<(String, bool)> {
    // When classifier is Regex, use the compiled pattern to do the
    // substitution with capture groups. For non-regex classifiers,
    // a rewrite means "replace the whole matching line with the
    // replacement string verbatim".
    let mut out: Vec<String> = Vec::with_capacity(input.lines().count());
    let mut fired = false;

    let compiled_regexes: Vec<Regex> = if matches!(classifier, Classifier::Regex) {
        values
            .iter()
            .map(|v| Regex::new(v).with_context(|| format!("compiling rewrite regex {v}")))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    for line in input.lines() {
        if !line_matches(line, classifier, values) {
            out.push(line.to_owned());
            continue;
        }
        fired = true;
        if matches!(classifier, Classifier::Regex) {
            let mut rewritten = line.to_owned();
            for re in &compiled_regexes {
                rewritten = re.replace_all(&rewritten, replacement).into_owned();
            }
            out.push(rewritten);
        } else {
            out.push(replacement.to_owned());
        }
    }

    Ok((out.join("\n"), fired))
}

// ---------------------------------------------------------------------------
// template-dedup primitive
// ---------------------------------------------------------------------------

fn apply_template_dedup(
    input: &str,
    normalize: &[NormalizeRule],
    min_run: usize,
    format: &str,
) -> Result<(String, bool)> {
    let min_run = min_run.max(2);

    // Compile all normalize rules once per call.
    let compiled: Vec<(Regex, &str)> = normalize
        .iter()
        .map(|n| {
            Regex::new(&n.regex)
                .with_context(|| format!("compiling normalize regex {}", n.regex))
                .map(|re| (re, n.replacement.as_str()))
        })
        .collect::<Result<Vec<_>>>()?;

    let normalize_line = |line: &str| -> String {
        let mut s = line.to_owned();
        for (re, rep) in &compiled {
            s = re.replace_all(&s, *rep).into_owned();
        }
        s
    };

    let lines: Vec<&str> = input.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    let mut fired = false;

    while i < lines.len() {
        // Skip blank/whitespace-only lines (respects the project-local
        // l1-template-dedup rule: never emit a [×N] with empty exemplar).
        if lines[i].trim().is_empty() {
            out.push(lines[i].to_owned());
            i = i.saturating_add(1);
            continue;
        }

        let template = normalize_line(lines[i]);
        let mut count = 1usize;
        while i.saturating_add(count) < lines.len() {
            let next = lines[i.saturating_add(count)];
            if next.trim().is_empty() {
                break;
            }
            if normalize_line(next) != template {
                break;
            }
            count = count.saturating_add(1);
        }

        if count >= min_run {
            let rendered = format
                .replace("{n}", &count.to_string())
                .replace("{exemplar}", lines[i]);
            out.push(rendered);
            fired = true;
            i = i.saturating_add(count);
        } else {
            out.push(lines[i].to_owned());
            i = i.saturating_add(1);
        }
    }

    Ok((out.join("\n"), fired))
}

// ---------------------------------------------------------------------------
// prefix-factor primitive
// ---------------------------------------------------------------------------

fn apply_prefix_factor(
    input: &str,
    min_share: f32,
    min_lines: usize,
    replacement: &str,
) -> (String, bool) {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < min_lines {
        return (input.to_owned(), false);
    }

    // Compute the longest common prefix character-wise across all lines.
    // Runs on chars (not bytes) to stay UTF-8 safe.
    let first = lines[0];
    let mut prefix_len_chars = first.chars().count();
    for &line in &lines[1..] {
        let mut common = 0usize;
        for (a, b) in first.chars().zip(line.chars()) {
            if a == b {
                common = common.saturating_add(1);
            } else {
                break;
            }
        }
        prefix_len_chars = prefix_len_chars.min(common);
        if prefix_len_chars == 0 {
            break;
        }
    }

    if prefix_len_chars < 2 {
        return (input.to_owned(), false);
    }

    // Compute share of lines that start with this prefix (they all do by
    // construction — this check is for future non-universal prefix
    // detection; hardcoded to 1.0 here).
    let share = 1.0_f32;
    if share < min_share {
        return (input.to_owned(), false);
    }

    let prefix: String = first.chars().take(prefix_len_chars).collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len().saturating_add(1));
    out.push(replacement.replace("{prefix}", &prefix));
    for &line in &lines {
        let stripped: String = line.chars().skip(prefix_len_chars).collect();
        out.push(format!("  {stripped}"));
    }

    (out.join("\n"), true)
}

// ---------------------------------------------------------------------------
// Invariant check: error signal preserved
// ---------------------------------------------------------------------------

#[allow(clippy::expect_used)]
static RE_ERROR_SIGNAL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(error:|ERROR|FAILED|panic:|Caused by|Traceback|Exception|fatal|warning:|E0[0-9]{3}:)",
    )
    .expect("error-signal regex must compile")
});

fn preserves_error_signal(before: &str, after: &str) -> bool {
    let count = |s: &str| RE_ERROR_SIGNAL.find_iter(s).count();
    // Allow MORE error matches post-transform (template collision),
    // reject strictly fewer — that would be a lost error.
    count(after) >= count(before)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loads_python_ruleset() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("rules")
            .join("stack_trace")
            .join("python.yaml");
        let file = load_rule_file(&path).expect("load python.yaml");
        assert_eq!(file.spec_version, "0.1");
        assert_eq!(file.language, "python");
        assert!(file.rules.len() >= 3);
        // Each rule must parse to FrameRun + CollapseRun in this POC.
        for r in &file.rules {
            assert!(matches!(r.pattern, Pattern::FrameRun { .. }));
            assert!(matches!(r.transform, Transform::CollapseRun { .. }));
        }
    }

    #[test]
    fn test_rejects_unknown_spec_version() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("bad.yaml");
        std::fs::write(&p, "spec_version: 1.5\nrules: []\n").unwrap();
        let err = load_rule_file(&p).unwrap_err();
        assert!(err.to_string().contains("0.X"));
    }

    #[test]
    fn test_frame_run_collapses_python_site_packages() {
        // 1 user frame + 5 site-packages frames (10 lines, 2 per frame)
        // + 1 user frame. collapse-run with min_run=3 should collapse
        // the middle 5 into one marker, preserving first + last of the run.
        let input = "\
Traceback (most recent call last):
  File \"/app/main.py\", line 10, in <module>
    run()
  File \"/usr/lib/python3/site-packages/django/core/handlers.py\", line 1, in a
    pass
  File \"/usr/lib/python3/site-packages/django/core/handlers.py\", line 2, in b
    pass
  File \"/usr/lib/python3/site-packages/django/core/handlers.py\", line 3, in c
    pass
  File \"/usr/lib/python3/site-packages/django/core/handlers.py\", line 4, in d
    pass
  File \"/usr/lib/python3/site-packages/django/core/handlers.py\", line 5, in e
    pass
  File \"/app/views.py\", line 20, in run
    crash()
ValueError: crashed";

        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("rules")
            .join("stack_trace")
            .join("python.yaml");
        let file = load_rule_file(&path).expect("load");
        let result = apply_rule_file(input, &file);

        // Error signal preserved (invariant #1).
        assert!(
            result.output.contains("ValueError: crashed"),
            "lost error line:\n{}",
            result.output
        );
        assert!(result.output.contains("Traceback"), "lost Traceback header");
        // User frames preserved (invariant #3).
        assert!(
            result.output.contains("/app/main.py"),
            "lost first user frame"
        );
        assert!(
            result.output.contains("/app/views.py"),
            "lost last user frame"
        );
        // Collapse actually happened.
        assert!(
            result.output.contains("frames omitted"),
            "no collapse marker:\n{}",
            result.output
        );
        assert!(!result.applied.is_empty(), "no rule fired; applied=[]");
    }

    #[test]
    fn test_does_not_collapse_below_min_run() {
        // Only 2 consecutive framework frames → below min_run=3 → no collapse.
        let input = "\
  File \"/app/main.py\", line 10, in main
    a()
  File \"/usr/lib/site-packages/x/y.py\", line 1, in a
    pass
  File \"/usr/lib/site-packages/x/y.py\", line 2, in b
    pass
  File \"/app/main.py\", line 20, in done
    ok()";
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("rules")
            .join("stack_trace")
            .join("python.yaml");
        let file = load_rule_file(&path).expect("load");
        let result = apply_rule_file(input, &file);

        assert!(
            !result.output.contains("frames omitted"),
            "should not collapse 2 frames"
        );
    }

    // --- line-match primitive ------------------------------------------

    #[test]
    fn test_line_match_delete_removes_matching_lines() {
        // Raw string + YAML single-quoted scalar: single backslashes
        // reach the regex engine verbatim. The earlier double-backslash
        // version bit us — YAML single-quotes do NOT process escapes.
        let rule_yaml = r#"
spec_version: 0.1
rules:
  - id: ansi.progress
    pattern:
      kind: line-match
      classifier: regex
      values: ['^\s*\[=+>?\s*\d+%']
    transform:
      kind: delete
    severity: lossy-safe
"#;
        let file: RuleFile = serde_yaml::from_str(rule_yaml).expect("parse");
        let input = "line one\n  [====>    50%]  done\nline two\n  [======>   75%]  more";
        let r = apply_rule_file(input, &file);
        assert!(r.output.contains("line one"));
        assert!(r.output.contains("line two"));
        assert!(!r.output.contains("50%"));
        assert!(!r.output.contains("75%"));
        assert_eq!(r.applied, vec!["ansi.progress"]);
    }

    #[test]
    fn test_line_match_rewrite_replaces_line_content() {
        let rule_yaml = r#"
spec_version: 0.1
rules:
  - id: redact.token
    pattern:
      kind: line-match
      classifier: regex
      values: ['Bearer [A-Za-z0-9._-]+']
    transform:
      kind: rewrite
      replacement: 'Bearer <redacted>'
    severity: lossy-safe
"#;
        let file: RuleFile = serde_yaml::from_str(rule_yaml).expect("parse");
        let input = "Authorization: Bearer eyJhbGciOi.abc.def\nContent-Type: application/json";
        let r = apply_rule_file(input, &file);
        assert!(r.output.contains("Bearer <redacted>"));
        assert!(!r.output.contains("eyJhbGciOi"));
        assert!(r.output.contains("application/json"));
    }

    // --- template-dedup primitive --------------------------------------

    #[test]
    fn test_template_dedup_collapses_repeated_warnings() {
        let rule_yaml = r#"
spec_version: 0.1
rules:
  - id: warn.dedup
    pattern:
      kind: template-dedup
      normalize:
        - regex: '\d+'
          replacement: '§'
    transform:
      kind: dedup
      min_run: 3
      format: '[×{n}] {exemplar}'
"#;
        let file: RuleFile = serde_yaml::from_str(rule_yaml).expect("parse");
        let input = "warning: retry 1 failed\nwarning: retry 2 failed\nwarning: retry 3 failed\nwarning: retry 4 failed\nok: done";
        let r = apply_rule_file(input, &file);
        assert!(
            r.output.contains("[×4] warning: retry 1 failed"),
            "dedup not applied: {}",
            r.output
        );
        assert!(r.output.contains("ok: done"));
    }

    #[test]
    fn test_template_dedup_skips_blank_lines() {
        // Regression guard: blank lines must never become `[×N]` exemplars.
        let rule_yaml = r#"
spec_version: 0.1
rules:
  - id: dedup.any
    pattern:
      kind: template-dedup
      normalize:
        - regex: '\d+'
          replacement: '§'
    transform:
      kind: dedup
      min_run: 2
      format: '[×{n}] {exemplar}'
"#;
        let file: RuleFile = serde_yaml::from_str(rule_yaml).expect("parse");
        let r = apply_rule_file("\n\nA\n", &file);
        // No `[×N]` with empty exemplar.
        assert!(
            !r.output.contains("[×2] \n"),
            "empty-exemplar regression: {:?}",
            r.output
        );
    }

    // --- prefix-factor primitive ---------------------------------------

    #[test]
    fn test_prefix_factor_extracts_common_prefix() {
        let rule_yaml = r#"
spec_version: 0.1
rules:
  - id: cargo.warn
    pattern:
      kind: prefix-factor
      min_share: 0.8
      min_lines: 3
    transform:
      kind: factor-prefix
      replacement: '[common prefix: {prefix}]'
"#;
        let file: RuleFile = serde_yaml::from_str(rule_yaml).expect("parse");
        let input = "warning: foo is dead\nwarning: bar is dead\nwarning: baz is dead\nwarning: qux is dead";
        let r = apply_rule_file(input, &file);
        assert!(
            r.output.contains("[common prefix:"),
            "prefix not factored: {}",
            r.output
        );
        assert!(!r.applied.is_empty());
    }

    #[test]
    fn test_invariant_preserves_error_lines() {
        // Even a rule authored to be aggressive should not drop errors;
        // invariant check rejects the transform post-hoc.
        let input = "error: build failed\n\
  File \"/site-packages/a/b.py\", line 1, in x\n    pass\n\
  File \"/site-packages/a/b.py\", line 2, in y\n    pass\n\
  File \"/site-packages/a/b.py\", line 3, in z\n    pass\n\
error: build failed twice";

        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("rules")
            .join("stack_trace")
            .join("python.yaml");
        let file = load_rule_file(&path).expect("load");
        let result = apply_rule_file(input, &file);

        // Both error lines must survive regardless of what the rule did
        // in the middle.
        assert!(result.output.contains("error: build failed\n"));
        assert!(result.output.contains("error: build failed twice"));
    }
}
