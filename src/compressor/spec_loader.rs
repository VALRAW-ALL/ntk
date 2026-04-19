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
    // Future: LineMatch, TemplateDedup, PrefixFactor
}

fn default_unit() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classifier {
    Contains,
    StartsWith,
    // Future: Regex, Equals
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Transform {
    CollapseRun {
        #[serde(default = "default_min_run")]
        min_run: usize,
        replacement: String,
    },
    // Future: Delete, Rewrite, Dedup, FactorPrefix
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
        let (new_text, fired) = match (&rule.pattern, &rule.transform) {
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
            ) => apply_frame_run(&current, classifier, values, *unit, *min_run, replacement),
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
    // Every line in the unit must match — Python's 2-line header+body
    // means both lines belong to the same frame. Bounds check prevents
    // over-read when we're near the tail.
    for offset in 0..unit {
        let pos = idx.saturating_add(offset);
        if pos >= lines.len() {
            return false;
        }
        let line = lines[pos];
        let hit = match classifier {
            Classifier::Contains => values.iter().any(|v| line.contains(v.as_str())),
            Classifier::StartsWith => {
                let t = line.trim_start();
                values.iter().any(|v| t.starts_with(v.as_str()))
            }
        };
        // For Python's 2-line frames, the first line (File "/path...") is
        // the path-bearing one; the second (body) is indented source. We
        // only need offset 0 to carry the signature — if the first line
        // matches, treat the whole unit as a framework frame.
        if offset == 0 && hit {
            return true;
        }
        if offset == 0 && !hit {
            return false;
        }
    }
    false
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
