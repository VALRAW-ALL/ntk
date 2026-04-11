// ---------------------------------------------------------------------------
// Etapa 24 — Snapshot tests: compression output regression guard
//
// These tests run the L1+L2 pipeline on every fixture and compare the output
// against a stored snapshot. Any change to the compression logic that alters
// output triggers a visible diff, which must be explicitly reviewed and
// approved with `cargo insta review`.
//
// First run (no snapshots yet):
//   cargo test --test snapshot_tests
//   cargo insta review          # inspect and approve
//
// Subsequent runs:
//   cargo test --test snapshot_tests   # passes if output is unchanged
//
// Force-update all snapshots (e.g. after intentional algorithm change):
//   INSTA_UPDATE=always cargo test --test snapshot_tests
// ---------------------------------------------------------------------------

use ntk::compressor::{layer1_filter, layer2_tokenizer};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn compress_fixture(name: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    let input = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));

    let l1 = layer1_filter::filter(&input);
    let l2 = layer2_tokenizer::process(&l1.output)
        .unwrap_or_else(|e| panic!("layer2 failed on {name}: {e}"));

    // Emit a header so the snapshot is self-documenting.
    format!(
        "# fixture: {name}\n\
         # tokens: {} -> {}\n\
         # lines_removed: {}\n\
         ---\n\
         {}",
        l2.original_tokens,
        l2.compressed_tokens,
        l1.lines_removed,
        l2.output,
    )
}

// ---------------------------------------------------------------------------
// Snapshot tests — one per fixture
// ---------------------------------------------------------------------------

#[test]
fn snapshot_cargo_test_output() {
    let result = compress_fixture("cargo_test_output.txt");
    insta::assert_snapshot!("cargo_test_output", result);
}

#[test]
fn snapshot_tsc_output() {
    let result = compress_fixture("tsc_output.txt");
    insta::assert_snapshot!("tsc_output", result);
}

#[test]
fn snapshot_vitest_output() {
    let result = compress_fixture("vitest_output.txt");
    insta::assert_snapshot!("vitest_output", result);
}

#[test]
fn snapshot_next_build_output() {
    let result = compress_fixture("next_build_output.txt");
    insta::assert_snapshot!("next_build_output", result);
}

#[test]
fn snapshot_docker_logs() {
    let result = compress_fixture("docker_logs.txt");
    insta::assert_snapshot!("docker_logs", result);
}

#[test]
fn snapshot_cargo_test_rtk_filtered() {
    let result = compress_fixture("cargo_test_rtk_filtered.txt");
    insta::assert_snapshot!("cargo_test_rtk_filtered", result);
}
