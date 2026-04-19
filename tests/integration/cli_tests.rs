// Etapa 13 — CLI integration tests via assert_cmd
//
// These tests invoke the compiled `ntk` binary and verify its behavior
// without requiring a running daemon.

use assert_cmd::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn ntk() -> Command {
    Command::cargo_bin("ntk").expect("ntk binary not found")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `ntk status` without a daemon should print a human-readable error message
/// (not panic or produce a stack trace).
#[test]
fn test_ntk_status_without_daemon() {
    let mut cmd = ntk();
    cmd.arg("status");
    // The command may succeed or fail, but it must NOT produce a panic/backtrace
    // and must produce some useful output.
    let output = cmd.output().expect("failed to run ntk status");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Must mention daemon, NTK, or "unreachable" in its output.
    assert!(
        combined.contains("daemon")
            || combined.contains("NTK")
            || combined.contains("unreachable")
            || combined.contains("health")
            || combined.contains("running"),
        "unexpected status output: {combined}"
    );
    // Must not contain a Rust panic backtrace.
    assert!(
        !combined.contains("thread 'main' panicked"),
        "status produced a panic: {combined}"
    );
}

/// `ntk init --show` (read-only) should not crash and should mention hook or config.
#[test]
fn test_ntk_init_show_does_not_crash() {
    let mut cmd = ntk();
    cmd.args(["init", "--show"]);
    let output = cmd.output().expect("failed to run ntk init --show");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Must not panic.
    assert!(
        !combined.contains("thread 'main' panicked"),
        "ntk init --show panicked: {combined}"
    );
    // Should mention hook or config or status keywords.
    assert!(
        combined.contains("Hook")
            || combined.contains("Config")
            || combined.contains("NTK")
            || combined.contains("not found")
            || combined.contains("✓")
            || combined.contains("✗"),
        "unexpected show output: {combined}"
    );
}

/// `ntk init --global` on a temp home dir should create the hook script and
/// a config.json without error.
#[test]
fn test_ntk_install_creates_hook() {
    let home = TempDir::new().expect("tempdir");
    let home_path = home.path();

    // NTK_HOME overrides dirs::home_dir() in the installer — works cross-platform.
    let mut cmd = ntk();
    cmd.args(["init", "--global", "--auto-patch"])
        .env("NTK_HOME", home_path)
        .env("NTK_SKIP_OLLAMA_INSTALL", "1");

    let output = cmd.output().expect("failed to run ntk init");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Command must succeed.
    assert!(
        output.status.success(),
        "ntk init failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Hook script must exist.
    #[cfg(target_os = "windows")]
    let hook_name = "ntk-hook.ps1";
    #[cfg(not(target_os = "windows"))]
    let hook_name = "ntk-hook.sh";

    let hook_path = home_path.join(".ntk").join("bin").join(hook_name);
    assert!(
        hook_path.exists(),
        "hook script not found at {}: stdout={stdout} stderr={stderr}",
        hook_path.display()
    );

    // Config must exist.
    let config_path = home_path.join(".ntk").join("config.json");
    assert!(
        config_path.exists(),
        "config.json not found at {}",
        config_path.display()
    );

    // settings.json should contain the NTK marker.
    // NTK_HOME also controls where editor settings are looked up.
    let settings = home_path.join(".claude").join("settings.json");
    let settings_content = std::fs::read_to_string(&settings).unwrap_or_default();
    assert!(
        settings_content.contains("ntk-hook"),
        "settings.json missing ntk-hook marker: {settings_content}"
    );
}

/// `ntk init --uninstall` should remove the NTK hook from a previously patched settings.json.
#[test]
fn test_ntk_uninstall_removes_hook() {
    let home = TempDir::new().expect("tempdir");
    let home_path = home.path();

    // First install.
    ntk()
        .args(["init", "--global", "--auto-patch"])
        .env("NTK_HOME", home_path)
        .env("NTK_SKIP_OLLAMA_INSTALL", "1")
        .output()
        .expect("install failed");

    // Verify hook was installed.
    let settings = home_path.join(".claude").join("settings.json");
    let before = std::fs::read_to_string(&settings).unwrap_or_default();
    assert!(
        before.contains("ntk-hook"),
        "hook not installed before uninstall test"
    );

    // Now uninstall.
    let output = ntk()
        .args(["init", "--global", "--uninstall"])
        .env("NTK_HOME", home_path)
        .env("NTK_SKIP_OLLAMA_INSTALL", "1")
        .output()
        .expect("uninstall failed");

    assert!(
        output.status.success(),
        "uninstall failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = std::fs::read_to_string(&settings).unwrap_or_default();
    assert!(
        !after.contains("ntk-hook"),
        "ntk-hook marker still present after uninstall: {after}"
    );
}

/// `ntk test-compress <fixture>` should print a compression ratio and not crash.
#[test]
fn test_ntk_test_compress_file() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cargo_test_output.txt");

    assert!(fixture.exists(), "fixture not found: {}", fixture.display());

    let output = ntk()
        .args(["test-compress", fixture.to_str().expect("fixture path")])
        .output()
        .expect("test-compress failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "test-compress failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Output must mention compression ratio.
    assert!(
        stdout.contains("Compression") || stdout.contains("%"),
        "expected compression ratio in output: {stdout}"
    );

    // Must not panic.
    assert!(
        !stdout.contains("thread 'main' panicked"),
        "test-compress panicked: {stdout}"
    );
}

/// `ntk bench --submit` must emit valid JSON with the expected shape.
#[test]
fn test_ntk_bench_submit_emits_valid_json() {
    let output = ntk()
        .args(["bench", "--runs", "1", "--submit"])
        .output()
        .expect("ntk bench --submit failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "bench --submit failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("not valid JSON: {e}\nraw:\n{stdout}");
    });
    assert!(parsed.get("ntk_version").is_some(), "missing ntk_version");
    assert!(parsed.get("os").is_some(), "missing os");
    assert!(parsed.get("arch").is_some(), "missing arch");
    assert!(parsed.get("gpu_backend").is_some(), "missing gpu_backend");
    let payloads = parsed
        .get("payloads")
        .and_then(|v| v.as_array())
        .expect("payloads array");
    assert!(!payloads.is_empty(), "payloads should not be empty");
    let first = &payloads[0];
    for field in &[
        "label",
        "tokens_in",
        "tokens_out_l2",
        "ratio_pct",
        "latency_us",
    ] {
        assert!(first.get(field).is_some(), "payload missing {field}");
    }
}

/// `ntk diff` must emit unified diff blocks for the chosen layer.
#[test]
fn test_ntk_diff_emits_unified_diff() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cargo_test_output.txt");
    assert!(fixture.exists(), "fixture not found: {}", fixture.display());

    let output = ntk()
        .args([
            "diff",
            fixture.to_str().expect("path"),
            "--layer",
            "l1",
            "--context",
            "1",
        ])
        .output()
        .expect("ntk diff failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ntk diff failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Header for the L1 comparison must appear.
    assert!(
        stdout.contains("Input vs L1"),
        "missing L1 header in diff output:\n{stdout}"
    );
    // At least one deletion line (L1 removes ANSI/progress/duplicates) —
    // cargo_test_output.txt reliably produces some.
    assert!(
        stdout.lines().any(|l| l.contains("- -")) || stdout.lines().any(|l| l.contains(" -  ")),
        "expected at least one '-' line (removal) in diff:\n{stdout}"
    );
    assert!(
        !stdout.contains("thread 'main' panicked"),
        "ntk diff panicked: {stdout}"
    );
}

/// `ntk test-compress --verbose` must emit sectioned breakdown headers
/// for the Input, L1, and L2 stages. L3 section only appears with --with-l3.
#[test]
fn test_ntk_test_compress_verbose_emits_sections() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cargo_test_output.txt");

    assert!(fixture.exists(), "fixture not found: {}", fixture.display());

    let output = ntk()
        .args([
            "test-compress",
            fixture.to_str().expect("fixture path"),
            "--verbose",
        ])
        .output()
        .expect("test-compress --verbose failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "test-compress --verbose failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    // Sectioned headers produced by print_verbose_section.
    for expected in &["┌─ Input", "┌─ L1 output", "┌─ L2 output", "preview (first"] {
        assert!(
            stdout.contains(expected),
            "verbose output missing marker {expected:?}:\n{stdout}"
        );
    }

    // Applied-rules breakdown must appear for both L1 and L2 (value depends
    // on fixture — we only check that the label is present).
    assert!(
        stdout.matches("Applied:").count() >= 2,
        "expected at least two 'Applied:' labels (L1 + L2):\n{stdout}"
    );

    // Non-verbose summary lines must NOT appear when --verbose is set.
    assert!(
        !stdout.contains("L1 lines removed:"),
        "non-verbose summary leaked into --verbose output:\n{stdout}"
    );

    assert!(
        !stdout.contains("thread 'main' panicked"),
        "test-compress --verbose panicked: {stdout}"
    );
}

/// `ntk gain` without a daemon should print a readable message (not panic).
#[test]
fn test_ntk_gain_format_rtk_compatible() {
    let output = ntk().arg("gain").output().expect("ntk gain failed");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Must not panic.
    assert!(
        !combined.contains("thread 'main' panicked"),
        "ntk gain panicked: {combined}"
    );

    // Should mention NTK or tokens or savings.
    assert!(
        combined.contains("NTK")
            || combined.contains("token")
            || combined.contains("daemon")
            || combined.contains("unreachable")
            || combined.contains("saved"),
        "unexpected gain output: {combined}"
    );
}
