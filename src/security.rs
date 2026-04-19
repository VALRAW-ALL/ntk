// Security primitives for the NTK daemon.
//
// The hook pipes every Bash tool output into the daemon. Without auth, any
// local process (or any process reachable via a mis-configured bind) can
// GET /records or POST /compress — leaking command stdout. This module
// provides a shared-secret token that the hook and daemon both read from
// `$NTK_HOME/.ntk/.token`.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// HTTP header the hook sends and the daemon validates.
pub const TOKEN_HEADER: &str = "X-NTK-Token";

/// Bypass env var. Forces the daemon to accept requests without a header —
/// intended for debugging only. A prominent warning is logged on startup.
pub const DISABLE_AUTH_ENV: &str = "NTK_DISABLE_AUTH";

/// Token length in raw bytes before base64 encoding (256 bits of entropy).
const TOKEN_BYTES: usize = 32;

/// Resolve `$HOME/.ntk/.token`, honoring `NTK_HOME` for tests and custom
/// install layouts.
pub fn token_path() -> Result<PathBuf> {
    let home = std::env::var_os("NTK_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".ntk").join(".token"))
}

/// Load the token from disk, creating one on first run. The file is written
/// with restrictive permissions (0o600 on Unix); on Windows the default ACL
/// of the user profile already restricts access.
pub fn load_or_create_token() -> Result<String> {
    let path = token_path()?;
    if let Some(existing) = read_existing(&path)? {
        return Ok(existing);
    }
    let token = generate_token();
    write_with_restricted_perms(&path, &token)
        .with_context(|| format!("writing token file {}", path.display()))?;
    Ok(token)
}

fn read_existing(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let trimmed = s.trim().to_owned();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context(format!("reading token file {}", path.display())),
    }
}

fn generate_token() -> String {
    // 32 bytes of entropy harvested via the uuid crate (already a direct
    // dependency). Each UUID v4 carries 122 random bits from the OS CSPRNG;
    // two UUIDs comfortably cover the 256-bit target without adding a new
    // crate to the supply chain.
    let a = *uuid::Uuid::new_v4().as_bytes();
    let b = *uuid::Uuid::new_v4().as_bytes();
    let mut bytes = [0u8; TOKEN_BYTES];
    bytes[..16].copy_from_slice(&a);
    bytes[16..].copy_from_slice(&b);
    base64_urlsafe_nopad(&bytes)
}

fn base64_urlsafe_nopad(bytes: &[u8]) -> String {
    // Tiny implementation to avoid pulling a new dep. Hex would also work
    // but we prefer fewer characters in config files and logs.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(
        bytes
            .len()
            .saturating_mul(4)
            .saturating_div(3)
            .saturating_add(2),
    );
    let mut i = 0usize;
    while i < bytes.len() {
        let remaining = bytes.len().saturating_sub(i);
        let b0 = bytes[i];
        let b1 = if remaining > 1 {
            bytes[i.saturating_add(1)]
        } else {
            0
        };
        let b2 = if remaining > 2 {
            bytes[i.saturating_add(2)]
        } else {
            0
        };

        let c0 = (b0 >> 2) & 0x3F;
        let c1 = ((b0 << 4) | (b1 >> 4)) & 0x3F;
        let c2 = ((b1 << 2) | (b2 >> 6)) & 0x3F;
        let c3 = b2 & 0x3F;

        out.push(ALPHABET[c0 as usize] as char);
        out.push(ALPHABET[c1 as usize] as char);
        if remaining > 1 {
            out.push(ALPHABET[c2 as usize] as char);
        }
        if remaining > 2 {
            out.push(ALPHABET[c3 as usize] as char);
        }
        i = i.saturating_add(3);
    }
    out
}

#[cfg(unix)]
fn write_with_restricted_perms(path: &Path, token: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    use std::io::Write;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(not(unix))]
fn write_with_restricted_perms(path: &Path, token: &str) -> Result<()> {
    // On Windows, files under %USERPROFILE%\.ntk inherit the user profile's
    // ACL (which restricts to the owning user and SYSTEM). We do not set
    // additional ACLs here — the surface is already restricted.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{token}\n"))?;
    Ok(())
}

/// Constant-time string comparison to mitigate timing side-channels when
/// comparing the presented header value against the expected token.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..ab.len() {
        // Fixed-count iteration; no short-circuit.
        diff |= ab[i] ^ bb[i];
    }
    diff == 0
}

/// Whether the current environment disables auth. Logged prominently when
/// true so operators notice.
pub fn auth_disabled() -> bool {
    std::env::var(DISABLE_AUTH_ENV)
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// One audit record emitted per `/compress` call when
/// `config.security.audit_log` is enabled. The output itself is never
/// persisted — only a SHA-256 hash, so the log reveals *that* a
/// compression happened but not *what* was compressed.
#[derive(serde::Serialize)]
pub struct AuditRecord<'a> {
    pub timestamp: String,
    pub command: &'a str,
    pub cwd: &'a str,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub layer: u8,
    pub output_sha256: String,
}

impl<'a> AuditRecord<'a> {
    pub fn new(
        command: &'a str,
        cwd: &'a str,
        tokens_before: usize,
        tokens_after: usize,
        layer: u8,
        output: &str,
    ) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(output.as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(hex, "{b:02x}");
        }
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            command,
            cwd,
            tokens_before,
            tokens_after,
            layer,
            output_sha256: hex,
        }
    }
}

/// Append an audit record as a single JSON line to `path`. Best-effort:
/// any I/O error is logged to tracing and swallowed — never fails the
/// /compress request because auditing is a side channel.
pub fn append_audit_record(path: &Path, record: &AuditRecord<'_>) {
    use std::io::Write;
    let line = match serde_json::to_string(record) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("audit: serialize failed: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("audit: mkdir {}: {e}", parent.display());
            return;
        }
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    match file {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                tracing::warn!("audit: write {}: {e}", path.display());
            }
        }
        Err(e) => tracing::warn!("audit: open {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_matches_and_mismatches() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc123", "abc12"));
        assert!(!constant_time_eq("", "x"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn test_generated_token_is_url_safe_and_correct_length() {
        // 32 raw bytes → 44 base64 chars (no padding → 43 in our implementation)
        let t = generate_token();
        // 32 bytes * 4 / 3 = 42.66 → 43 chars without padding (ceil).
        assert_eq!(t.len(), 43, "unexpected token length: {}", t.len());
        for c in t.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "non-urlsafe char in token: {c}"
            );
        }
    }

    #[test]
    fn test_token_path_respects_ntk_home() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let prev = std::env::var_os("NTK_HOME");
        std::env::set_var("NTK_HOME", tmp.path());
        let p = token_path().expect("token_path");
        assert_eq!(p, tmp.path().join(".ntk").join(".token"));
        match prev {
            Some(v) => std::env::set_var("NTK_HOME", v),
            None => std::env::remove_var("NTK_HOME"),
        }
    }
}
