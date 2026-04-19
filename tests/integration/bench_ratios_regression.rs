// ---------------------------------------------------------------------------
// Deterministic bench-ratio regression guard (#22)
//
// GitHub runners produce noisy latency numbers, so `cargo bench` on CI is
// informational only. *Ratios*, on the other hand, are a pure function of
// the input bytes + L1/L2 code — identical on every machine. This test
// iterates every `bench/fixtures/*.txt`, runs L1+L2 in-process, and
// asserts the compression ratio stays above the `min_ratio` floor
// documented in the matching `.meta.json`.
//
// When a PR tightens a classifier, bump the fixture's `min_ratio` as part
// of the same commit — the CI failure here tells you which fixture drifted.
//
// Fixtures with `expected_layer: 3` (or ≥ 3) require L3 inference, which
// is not deterministic on CI. Those are skipped here and covered by the
// offline `bench/replay.ps1` experiment instead.
// ---------------------------------------------------------------------------

use ntk::compressor::{layer1_filter, layer2_tokenizer};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize)]
struct Meta {
    #[serde(default)]
    category: String,
    #[serde(default)]
    min_ratio: f64,
    #[serde(default)]
    expected_layer: u8,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bench")
        .join("fixtures")
}

fn load_meta(meta_path: &Path) -> Option<Meta> {
    let text = fs::read_to_string(meta_path).ok()?;
    serde_json::from_str(&text).ok()
}

fn measure_ratio(input: &str) -> f64 {
    let original = match layer2_tokenizer::count_tokens(input) {
        Ok(n) => n,
        Err(_) => return 0.0,
    };
    if original == 0 {
        return 0.0;
    }
    let l1 = layer1_filter::filter(input);
    let l2 = match layer2_tokenizer::process(&l1.output) {
        Ok(r) => r,
        Err(_) => return 0.0,
    };
    let saved = original.saturating_sub(l2.compressed_tokens);
    (saved as f64) / (original as f64)
}

#[test]
fn all_bench_fixtures_meet_their_min_ratio_floor() {
    let dir = fixtures_dir();
    assert!(
        dir.is_dir(),
        "bench/fixtures/ not found at {}",
        dir.display()
    );

    let entries: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir({}): {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("txt"))
        .collect();

    assert!(
        !entries.is_empty(),
        "no *.txt fixtures under {}",
        dir.display()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for fx in &entries {
        let stem = fx
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("<unnamed>")
            .to_owned();
        let meta_path = fx.with_extension("meta.json");
        let Some(meta) = load_meta(&meta_path) else {
            // No meta → nothing to check; skip silently.
            continue;
        };

        // L3 fixtures are non-deterministic across machines (Ollama output
        // varies); trust the offline replay for those.
        if meta.expected_layer >= 3 {
            continue;
        }
        // min_ratio=0 marks "no floor expected" (e.g. already_short).
        if meta.min_ratio <= 0.0 {
            continue;
        }

        let input = match fs::read_to_string(fx) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{stem}: cannot read fixture ({e})"));
                continue;
            }
        };

        let ratio = measure_ratio(&input);
        checked = checked.saturating_add(1);

        if ratio < meta.min_ratio {
            failures.push(format!(
                "{stem} [{}]: measured {:.3} < min_ratio {:.3}",
                meta.category, ratio, meta.min_ratio
            ));
        }
    }

    assert!(
        checked > 0,
        "no deterministic fixtures were checked — did the meta.json schema change?"
    );
    assert!(
        failures.is_empty(),
        "ratio regression(s) detected ({} fixture(s) below floor):\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn bench_fixtures_have_meta_for_ratio_gate() {
    // Every *.txt should have a matching *.meta.json so regressions
    // can be attributed to a named fixture. A missing meta is a bug in
    // the fixture-generation script, not a test failure — warn but
    // don't break CI yet.
    let dir = fixtures_dir();
    let mut missing: Vec<String> = Vec::new();
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir: {e}")) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("txt") {
            continue;
        }
        let meta = path.with_extension("meta.json");
        if !meta.exists() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                missing.push(name.to_owned());
            }
        }
    }
    assert!(
        missing.is_empty(),
        "fixtures without .meta.json (add one via bench/generate_fixtures.ps1):\n  - {}",
        missing.join("\n  - ")
    );
}
