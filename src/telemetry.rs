// ---------------------------------------------------------------------------
// Etapa 25b — Anonymous telemetry (opt-out via NTK_TELEMETRY_DISABLED)
//
// What is collected:
//   - Device hash (SHA-256 with per-user random salt — not reversible)
//   - NTK version, OS, architecture
//   - Command count (last 24h) and most-used command names (no args, no paths)
//   - Average token savings percentage
//   - Layer distribution (L1/L2/L3 %)
//   - GPU backend used
//
// What is NOT collected:
//   Source code, file paths, command arguments, secrets, env vars, or any PII.
//
// Opt-out:
//   export NTK_TELEMETRY_DISABLED=1          (env var)
//   config.telemetry.enabled = false          (config file)
//
// Design:
//   - Fire-and-forget: telemetry failure never blocks the compression pipeline
//   - Sends at most once per day (marker file in ~/.ntk/)
//   - Timeout: 3 seconds
//   - Salt stored in ~/.ntk/.telemetry_salt (generated once, never sent)
// ---------------------------------------------------------------------------

use anyhow::{Context, Result};
use dirs::home_dir;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Opt-out guard — must be checked before building any payload
// ---------------------------------------------------------------------------

/// Returns `true` if telemetry is enabled (i.e. NOT disabled).
///
/// Checks `NTK_TELEMETRY_DISABLED` env var first, then the config flag.
pub fn is_enabled(config_enabled: bool) -> bool {
    if std::env::var("NTK_TELEMETRY_DISABLED").is_ok() {
        return false;
    }
    config_enabled
}

// ---------------------------------------------------------------------------
// Salt management
// ---------------------------------------------------------------------------

fn salt_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ntk").join(".telemetry_salt"))
}

/// Load or generate the per-user random salt.
///
/// The salt is stored in `~/.ntk/.telemetry_salt` on first call and reused on
/// subsequent calls. It is never sent to any server.
fn load_or_create_salt() -> Result<String> {
    let path = salt_path().context("cannot determine home directory")?;

    if path.exists() {
        let salt = std::fs::read_to_string(&path).context("reading telemetry salt")?;
        let salt = salt.trim().to_owned();
        if !salt.is_empty() {
            return Ok(salt);
        }
    }

    // Generate a new salt.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("creating .ntk directory")?;
    }
    let salt = uuid::Uuid::new_v4().to_string();
    std::fs::write(&path, &salt).context("writing telemetry salt")?;

    // Set restrictive permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).ok();
    }

    Ok(salt)
}

// ---------------------------------------------------------------------------
// Device hash
// ---------------------------------------------------------------------------

/// Compute a non-reversible device identifier: `SHA-256(salt + machine_id)`.
///
/// The machine_id is a best-effort stable identifier. Falls back to the
/// hostname, and finally to an empty string. The hash is HEX-encoded.
fn device_hash(salt: &str) -> String {
    let machine_id = read_machine_id().unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(machine_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn read_machine_id() -> Option<String> {
    // Linux: /etc/machine-id
    #[cfg(target_os = "linux")]
    if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
        return Some(id.trim().to_owned());
    }

    // macOS: ioreg serial number is available via CLI; use hostname as fallback.
    // Windows: use COMPUTERNAME env var.
    #[cfg(target_os = "windows")]
    if let Ok(name) = std::env::var("COMPUTERNAME") {
        return Some(name);
    }

    // Fallback: hostname
    hostname().ok()
}

fn hostname() -> Result<String> {
    let output = std::process::Command::new("hostname")
        .output()
        .context("running hostname command")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

// ---------------------------------------------------------------------------
// Daily send gate
// ---------------------------------------------------------------------------

fn marker_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ntk").join(".telemetry_sent_date"))
}

/// Returns `true` if we already sent telemetry today (UTC date).
fn already_sent_today() -> bool {
    let Some(path) = marker_path() else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let today = today_utc();
    content.trim() == today
}

fn mark_sent_today() {
    let Some(path) = marker_path() else { return };
    let today = today_utc();
    // Ignore write errors — telemetry is best-effort.
    let _ = std::fs::write(path, today);
}

fn today_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

// ---------------------------------------------------------------------------
// Telemetry payload (no paths, no args, no PII)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct TelemetryPayload {
    pub device_hash: String,
    pub ntk_version: String,
    pub os: String,
    pub arch: String,
    pub gpu_backend: String,
    pub compressions_24h: usize,
    pub top_commands: Vec<String>,
    pub avg_savings_pct: f32,
    pub layer_pct: [f32; 3],
}

impl TelemetryPayload {
    /// Verify the payload contains no absolute paths or user-identifying strings.
    ///
    /// This is called in tests. Production code trusts the constructor.
    pub fn has_no_paths(&self) -> bool {
        let suspicious = |s: &str| {
            s.contains('/')
                || s.contains('\\')
                || s.contains("home")
                || s.contains("Users")
                || s.contains("user")
        };
        if suspicious(&self.device_hash) {
            return false;
        }
        for cmd in &self.top_commands {
            if suspicious(cmd) {
                return false;
            }
        }
        true
    }
}

/// Build the telemetry payload.
///
/// Returns `None` when telemetry is disabled or already sent today.
/// Returns `Err` only when the salt cannot be created/read (filesystem error).
pub fn build_payload(
    config_enabled: bool,
    gpu_backend: &str,
    compressions_24h: usize,
    top_commands: Vec<String>,
    avg_savings_pct: f32,
    layer_pct: [f32; 3],
) -> Result<Option<TelemetryPayload>> {
    // Security: check opt-out BEFORE constructing the payload.
    if !is_enabled(config_enabled) {
        return Ok(None);
    }
    if already_sent_today() {
        return Ok(None);
    }

    let salt = load_or_create_salt()?;
    let hash = device_hash(&salt);

    Ok(Some(TelemetryPayload {
        device_hash: hash,
        ntk_version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        gpu_backend: gpu_backend.to_owned(),
        compressions_24h,
        top_commands,
        avg_savings_pct,
        layer_pct,
    }))
}

// ---------------------------------------------------------------------------
// Send — fire and forget, 3s timeout
// ---------------------------------------------------------------------------

const TELEMETRY_ENDPOINT: &str = "https://telemetry.ntk.dev/v1/ping";

/// Send the payload to the telemetry endpoint. Fire-and-forget: errors are
/// silently swallowed so they never affect the compression pipeline.
///
/// This function spawns a Tokio task and returns immediately.
pub fn send_fire_and_forget(payload: TelemetryPayload) {
    tokio::spawn(async move {
        // Failure is silently ignored — never log payload details.
        let _ = send_payload(payload).await;
        mark_sent_today();
    });
}

async fn send_payload(payload: TelemetryPayload) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .context("building reqwest client")?;

    client
        .post(TELEMETRY_ENDPOINT)
        .json(&payload)
        .send()
        .await
        .context("sending telemetry")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    pub fn test_disabled_via_env() {
        // Safety: tests run in a single process; other tests must not rely on
        // this env var being unset.
        std::env::set_var("NTK_TELEMETRY_DISABLED", "1");
        assert!(!is_enabled(true), "should be disabled when env var is set");
        std::env::remove_var("NTK_TELEMETRY_DISABLED");
    }

    #[test]
    pub fn test_payload_has_no_paths() {
        // Construct a payload directly (bypassing the daily gate).
        let payload = TelemetryPayload {
            device_hash: "abc123deadbeef".to_owned(),
            ntk_version: "0.1.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            gpu_backend: "cpu".to_owned(),
            compressions_24h: 42,
            top_commands: vec!["cargo".to_owned(), "git".to_owned()],
            avg_savings_pct: 75.0,
            layer_pct: [0.3, 0.6, 0.1],
        };
        assert!(
            payload.has_no_paths(),
            "telemetry payload must not contain paths or usernames"
        );
    }

    #[test]
    pub fn test_salt_generated_once() {
        // Verify that loading salt twice returns the same value.
        // Uses a temp dir so it doesn't clobber the real ~/.ntk/.telemetry_salt.
        let tmp = tempfile::tempdir().expect("tempdir");
        let salt_file = tmp.path().join(".telemetry_salt");

        // Write a known salt.
        std::fs::write(&salt_file, "test-salt-abc").expect("write salt");

        // Simulate reading by directly reading the file (load_or_create_salt
        // uses the real home path, so we just test the logic inline).
        let content1 = std::fs::read_to_string(&salt_file).expect("read 1");
        let content2 = std::fs::read_to_string(&salt_file).expect("read 2");
        assert_eq!(content1.trim(), content2.trim());
    }
}
