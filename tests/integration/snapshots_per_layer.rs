// ---------------------------------------------------------------------------
// Per-layer snapshot tests — one snapshot per (fixture × layer) pair.
//
// The full-pipeline snapshots in `snapshot_tests.rs` guarantee the end-to-end
// output doesn't drift, but when a ratio regresses they don't tell us *which*
// layer caused it. These per-layer snapshots make regressions point at the
// exact stage: if only `layer1__<fixture>.snap` diffs on a PR, the change
// is L1-only; if `layer2__<fixture>.snap` also diffs, it's a downstream
// effect of the L1 change.
//
// First run (no snapshots yet):
//   INSTA_UPDATE=always cargo test --test snapshots_per_layer
//   # Then review the generated .snap files before committing.
//
// Subsequent runs:
//   cargo test --test snapshots_per_layer   # passes if unchanged
// ---------------------------------------------------------------------------

use ntk::compressor::{layer1_filter, layer2_tokenizer};

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn read_fixture(name: &str) -> String {
    let path = fixture_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

/// L1-only output with a small header so the snapshot is self-documenting.
/// Applied rules are serialized in a stable order (they are pushed in the
/// same order every run, per the layer1_filter pipeline).
fn layer1_snapshot(name: &str) -> String {
    let input = read_fixture(name);
    let l1 = layer1_filter::filter(&input);
    format!(
        "# fixture: {name}\n\
         # layer: 1\n\
         # lines_removed: {}\n\
         # applied_rules: {:?}\n\
         # rtk_pre_filtered: {}\n\
         ---\n\
         {}",
        l1.lines_removed, l1.applied_rules, l1.rtk_pre_filtered, l1.output,
    )
}

/// L2-only output (L2 receives the L1 output as input, as it does in prod).
fn layer2_snapshot(name: &str) -> String {
    let input = read_fixture(name);
    let l1 = layer1_filter::filter(&input);
    let l2 = layer2_tokenizer::process(&l1.output)
        .unwrap_or_else(|e| panic!("layer2 failed on {name}: {e}"));
    format!(
        "# fixture: {name}\n\
         # layer: 2\n\
         # tokens_in: {}\n\
         # tokens_out: {}\n\
         # applied_rules: {:?}\n\
         ---\n\
         {}",
        l2.original_tokens, l2.compressed_tokens, l2.applied_rules, l2.output,
    )
}

// ---------------------------------------------------------------------------
// Layer 1 snapshots — one per fixture
// ---------------------------------------------------------------------------

#[test]
fn layer1_cargo_test_output() {
    insta::assert_snapshot!(
        "layer1__cargo_test_output",
        layer1_snapshot("cargo_test_output.txt")
    );
}

#[test]
fn layer1_tsc_output() {
    insta::assert_snapshot!("layer1__tsc_output", layer1_snapshot("tsc_output.txt"));
}

#[test]
fn layer1_vitest_output() {
    insta::assert_snapshot!(
        "layer1__vitest_output",
        layer1_snapshot("vitest_output.txt")
    );
}

#[test]
fn layer1_next_build_output() {
    insta::assert_snapshot!(
        "layer1__next_build_output",
        layer1_snapshot("next_build_output.txt")
    );
}

#[test]
fn layer1_docker_logs() {
    insta::assert_snapshot!("layer1__docker_logs", layer1_snapshot("docker_logs.txt"));
}

#[test]
fn layer1_cargo_test_rtk_filtered() {
    insta::assert_snapshot!(
        "layer1__cargo_test_rtk_filtered",
        layer1_snapshot("cargo_test_rtk_filtered.txt")
    );
}

// ---------------------------------------------------------------------------
// Layer 2 snapshots — same fixture set, L2-only output
// ---------------------------------------------------------------------------

#[test]
fn layer2_cargo_test_output() {
    insta::assert_snapshot!(
        "layer2__cargo_test_output",
        layer2_snapshot("cargo_test_output.txt")
    );
}

#[test]
fn layer2_tsc_output() {
    insta::assert_snapshot!("layer2__tsc_output", layer2_snapshot("tsc_output.txt"));
}

#[test]
fn layer2_vitest_output() {
    insta::assert_snapshot!(
        "layer2__vitest_output",
        layer2_snapshot("vitest_output.txt")
    );
}

#[test]
fn layer2_next_build_output() {
    insta::assert_snapshot!(
        "layer2__next_build_output",
        layer2_snapshot("next_build_output.txt")
    );
}

#[test]
fn layer2_docker_logs() {
    insta::assert_snapshot!("layer2__docker_logs", layer2_snapshot("docker_logs.txt"));
}

#[test]
fn layer2_cargo_test_rtk_filtered() {
    insta::assert_snapshot!(
        "layer2__cargo_test_rtk_filtered",
        layer2_snapshot("cargo_test_rtk_filtered.txt")
    );
}
