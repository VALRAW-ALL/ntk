// ---------------------------------------------------------------------------
// Integration test: every shipped ruleset × every bench fixture
//
// The POC's big claim is "four primitives cover every mainstream
// language". This suite validates it operationally: load every YAML
// rule file under `rules/`, apply each to the closest-matching
// fixture under `bench/fixtures/`, and assert:
//
//   1. The loader never panics — any byte sequence in a fixture can
//      be fed through any ruleset and produces a valid result.
//   2. The invariant check holds: error / panic / Traceback / FAILED
//      markers are never fewer in the output than the input.
//   3. Framework-collapse markers appear when the fixture actually
//      contains runs of framework frames (spot-checked per language).
//
// The suite is deliberately lenient on compression ratio — a ruleset
// shipped here is allowed to produce "no change" on a fixture it
// doesn't target. What it's NOT allowed to do is drop errors or crash.
// ---------------------------------------------------------------------------

use ntk::compressor::spec_loader::{apply_rule_file, load_rule_file};
use std::fs;
use std::path::{Path, PathBuf};

fn rules_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules")
}

fn bench_fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bench")
        .join("fixtures")
}

fn all_rule_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for category_dir in walk_dirs(&rules_root()) {
        for entry in fs::read_dir(&category_dir).unwrap_or_else(|e| panic!("{e}")) {
            let entry = entry.unwrap_or_else(|e| panic!("{e}"));
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if let Ok(dir) = fs::read_dir(root) {
        for entry in dir.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out
}

fn fixture(name: &str) -> Option<String> {
    let p = bench_fixtures_root().join(name);
    fs::read_to_string(p).ok()
}

// Intentionally no local error-count regex — the spec_loader applies
// its own invariant check internally and surfaces any rejection via
// `ApplyResult.invariant_rejected`. The integration test below asserts
// on that signal directly, which guarantees parity with the runtime.

/// Loads every YAML rule file in rules/ and confirms the parser accepts
/// them. A regression in the schema would surface here as a parse error.
#[test]
fn every_shipped_ruleset_parses() {
    let files = all_rule_files();
    assert!(
        !files.is_empty(),
        "no rule files found at {}",
        rules_root().display()
    );

    let mut failures = Vec::new();
    for path in &files {
        if let Err(e) = load_rule_file(path) {
            failures.push(format!("{}: {e}", path.display()));
        }
    }

    assert!(
        failures.is_empty(),
        "ruleset parse failures:\n  - {}",
        failures.join("\n  - ")
    );
}

/// Every ruleset must be safe to apply to every fixture: no panics,
/// no rules rejected by the built-in invariant check. The spec_loader
/// runtime already enforces `preserve_errors` post-hoc; we just assert
/// that NO rule in the shipped corpus triggers that rejection against
/// any fixture. If a rule does trigger rejection, either the rule is
/// too aggressive or the invariant regex is still wrong — either way,
/// the corpus isn't shippable as-is.
#[test]
fn every_ruleset_against_every_fixture_respects_invariants() {
    let rule_paths = all_rule_files();
    let mut fixture_names: Vec<String> = fs::read_dir(bench_fixtures_root())
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("txt"))
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(|s| s.to_owned()))
        .collect();
    fixture_names.sort();

    let mut failures = Vec::new();
    let mut pairs_checked = 0usize;

    for rule_path in &rule_paths {
        let rule_file = match load_rule_file(rule_path) {
            Ok(f) => f,
            Err(_) => continue, // covered by every_shipped_ruleset_parses
        };
        for name in &fixture_names {
            let input = match fixture(name) {
                Some(s) => s,
                None => continue,
            };

            let result = apply_rule_file(&input, &rule_file);
            pairs_checked = pairs_checked.saturating_add(1);

            if !result.invariant_rejected.is_empty() {
                failures.push(format!(
                    "{} × {}: rejected rules = {:?}",
                    rule_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?"),
                    name,
                    result.invariant_rejected
                ));
            }
        }
    }

    assert!(pairs_checked > 0, "no rule×fixture pairs exercised");
    assert!(
        failures.is_empty(),
        "invariant rejections across {} pairs:\n  - {}",
        pairs_checked,
        failures.join("\n  - ")
    );
}

/// Spot-check: the Python ruleset on the Python Django fixture should
/// actually produce a visible collapse marker. If a future schema
/// change breaks collapse without tripping the error-preservation
/// invariant, this catches it.
#[test]
fn python_ruleset_collapses_on_django_fixture() {
    let path = rules_root().join("stack_trace").join("python.yaml");
    let rule_file = load_rule_file(&path).expect("load python.yaml");
    let input =
        fixture("python_django_trace.txt").expect("python_django_trace.txt fixture present");
    let result = apply_rule_file(&input, &rule_file);
    assert!(
        result.output.contains("frames omitted"),
        "expected a collapse marker on python_django_trace:\n{}",
        result.output
    );
}

/// Spot-check: Java ruleset on the Java fixture.
#[test]
fn java_ruleset_collapses_on_java_fixture() {
    let path = rules_root().join("stack_trace").join("java.yaml");
    let rule_file = load_rule_file(&path).expect("load java.yaml");
    let input = fixture("stack_trace_java.txt").expect("stack_trace_java.txt fixture present");
    let result = apply_rule_file(&input, &rule_file);
    assert!(
        result.output.contains("frames omitted"),
        "expected a collapse marker on stack_trace_java:\n{}",
        result.output
    );
}

/// Spot-check: PHP ruleset on the Symfony fixture.
#[test]
fn php_ruleset_collapses_on_symfony_fixture() {
    let path = rules_root().join("stack_trace").join("php.yaml");
    let rule_file = load_rule_file(&path).expect("load php.yaml");
    let input = fixture("php_symfony_trace.txt").expect("php_symfony_trace.txt fixture present");
    let result = apply_rule_file(&input, &rule_file);
    assert!(
        result.output.contains("frames omitted"),
        "expected a collapse marker on php_symfony_trace:\n{}",
        result.output
    );
}
