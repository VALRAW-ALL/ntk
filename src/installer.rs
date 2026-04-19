use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

use crate::output::terminal as term;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTarget {
    ClaudeCode,
    OpenCode,
    /// Cursor — registers `ntk mcp-server` in `~/.cursor/mcp.json`
    /// under the `mcpServers` key (flat command/args shape).
    Cursor,
    /// Zed — registers `ntk mcp-server` in the user's Zed settings.json
    /// under `context_servers` (command-object shape with
    /// `command.path`, `command.args`, `command.env`).
    Zed,
    /// Continue — registers `ntk mcp-server` in `~/.continue/config.json`
    /// under `mcpServers` (array of named server objects, distinct from
    /// Cursor's object-keyed-by-name).
    Continue,
}

impl EditorTarget {
    /// Returns true when the editor uses MCP rather than a PostToolUse
    /// hook. Drives the install-path split between `inject_ntk_hook`
    /// and the MCP-specific inject functions.
    pub fn uses_mcp(self) -> bool {
        matches!(
            self,
            EditorTarget::Cursor | EditorTarget::Zed | EditorTarget::Continue
        )
    }
}

pub struct Installer {
    pub editor: EditorTarget,
    pub auto_patch: bool,
    pub hook_only: bool,
}

// ---------------------------------------------------------------------------
// Hook JSON block injected into settings.json
// ---------------------------------------------------------------------------

const NTK_HOOK_MARKER: &str = "ntk-hook";

fn hook_command() -> String {
    #[cfg(target_os = "windows")]
    {
        // PowerShell does NOT expand `~` when receiving a path via the `-File`
        // command-line argument — expand the home directory at install time so
        // the stored command is always an absolute path.
        if let Ok(home) = home_dir() {
            let ps1 = home.join(".ntk").join("bin").join("ntk-hook.ps1");
            return format!("powershell -NoProfile -File \"{}\"", ps1.display());
        }
        // Fallback if home dir is unavailable (should never happen in practice).
        "powershell -NoProfile -File \"%USERPROFILE%\\.ntk\\bin\\ntk-hook.ps1\"".to_owned()
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Unix shells expand `~` before passing the argument — safe to use here.
        "~/.ntk/bin/ntk-hook.sh".to_owned()
    }
}

// ---------------------------------------------------------------------------
// Step outcome — drives the summary icons
// ---------------------------------------------------------------------------

#[derive(PartialEq)]
enum StepOutcome {
    Ok,
    Warn,
    #[allow(dead_code)]
    Err,
}

impl StepOutcome {
    fn icon(&self) -> &'static str {
        match self {
            StepOutcome::Ok => "🟢",
            StepOutcome::Warn => "🟡",
            StepOutcome::Err => "🔴",
        }
    }
}

// ---------------------------------------------------------------------------
// impl Installer
// ---------------------------------------------------------------------------

impl Installer {
    /// Execute the installation.
    pub fn run(&self) -> Result<()> {
        let ntk_bin_dir = ntk_bin_dir()?;

        println!(
            "\n{}{}  Installing NTK globally…{}\n",
            term::bold(),
            term::bright_cyan(),
            term::reset()
        );

        // Collect per-step status for the final summary.
        // (label, detail, StepOutcome)
        let mut summary: Vec<(&str, String, StepOutcome)> = Vec::new();

        // Step 1: Create ~/.ntk/bin/
        let sp = term::Spinner::start("Creating ~/.ntk/bin …");
        match ensure_dir(&ntk_bin_dir) {
            Ok(()) => sp.finish_ok("directory ready"),
            Err(e) => {
                sp.finish_err(&e.to_string());
                return Err(e);
            }
        }

        // Step 2: Copy NTK binary to ~/.ntk/bin/ntk[.exe]
        let sp = term::Spinner::start("Installing NTK binary …");
        match install_ntk_binary(&ntk_bin_dir) {
            Ok(msg) => {
                sp.finish_ok(&msg);
                summary.push(("Binary ", msg, StepOutcome::Ok));
            }
            Err(e) => {
                sp.finish_err(&e.to_string());
                return Err(e);
            }
        }

        // Step 3: Copy hook script
        let hook_path = ntk_bin_dir.join(hook_script_name());
        let sp = term::Spinner::start("Installing hook script …");
        match copy_hook_script(&ntk_bin_dir) {
            Ok(()) => {
                let msg = hook_path.display().to_string();
                sp.finish_ok(&msg);
                summary.push(("Hook   ", msg, StepOutcome::Ok));
            }
            Err(e) => {
                sp.finish_err(&e.to_string());
                return Err(e);
            }
        }

        // Step 4: Add ~/.ntk/bin to user PATH (non-fatal — user can update manually)
        let sp = term::Spinner::start("Updating PATH …");
        match add_dir_to_path(&ntk_bin_dir) {
            Ok(msg) => sp.finish_ok(&msg),
            Err(e) => {
                let msg = format!(
                    "skipped — {e}  (add manually: export PATH=\"$PATH:{}\")",
                    ntk_bin_dir.display()
                );
                sp.finish_warn(&msg);
            }
        }

        // Step 5: Patch editor settings.json
        // (Model backend / Ollama installation is handled separately by `ntk model setup`.)
        let settings_path = editor_settings_path(self.editor)?;
        let sp = term::Spinner::start("Patching editor settings …");
        match patch_settings(&settings_path, self.auto_patch, self.editor) {
            Ok(()) => {
                let msg = settings_path.display().to_string();
                sp.finish_ok(&msg);
                summary.push(("Editor ", msg, StepOutcome::Ok));
            }
            Err(e) => {
                sp.finish_err(&e.to_string());
                return Err(e);
            }
        }

        // Step 6: Create default config (unless --hook-only)
        let config_path = global_config_path()?;
        if !self.hook_only {
            let sp = term::Spinner::start("Creating config …");
            match create_default_config() {
                Ok(msg) => {
                    sp.finish_ok(&msg);
                    summary.push(("Config ", msg, StepOutcome::Ok));
                }
                Err(e) => {
                    sp.finish_err(&e.to_string());
                    return Err(e);
                }
            }
        }

        // Summary
        let has_warn = summary.iter().any(|(_, _, o)| *o == StepOutcome::Warn);
        println!();
        if has_warn {
            println!(
                "  {}{}✓ NTK installed (with warnings){}",
                term::bold(),
                term::bright_yellow(),
                term::reset()
            );
        } else {
            println!(
                "  {}{}✓ NTK installed successfully!{}",
                term::bold(),
                term::bright_green(),
                term::reset()
            );
        }
        println!();

        // Per-step status line: icon + label + detail
        let bin_path = ntk_bin_dir.join(ntk_binary_name());
        // Always show binary and hook even if we didn't push them (edge case safety).
        let _ = (&bin_path, &hook_path, &config_path, &settings_path);

        for (label, detail, outcome) in &summary {
            println!(
                "  {} {}{}{}  {}",
                outcome.icon(),
                term::bold(),
                label,
                term::reset(),
                detail
            );
        }

        println!();
        println!(
            "  {}ℹ  Layers 1+2 (fast compression) are active by default.{}",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}   Run `ntk model setup` to configure Layer 3 (AI inference).{}",
            term::dim(),
            term::reset()
        );
        println!(
            "  {}💡 Open a new terminal, then run `ntk start` to start the daemon.{}",
            term::dim(),
            term::reset()
        );
        println!();
        Ok(())
    }

    /// Show current installation status without modifying anything.
    pub fn show_status(&self) -> Result<()> {
        term::print_header(
            "NTK Installation Status",
            "───────────────────────────────────────",
        );
        println!();

        let hook_path = ntk_bin_dir()?.join(hook_script_name());
        print_status_line("🔗 Hook script ", &hook_path);

        let config_path = global_config_path()?;
        print_status_line("⚙  Config      ", &config_path);

        let settings_path = editor_settings_path(self.editor)?;
        let hook_installed = settings_path.exists()
            && std::fs::read_to_string(&settings_path)
                .map(|s| s.contains(NTK_HOOK_MARKER))
                .unwrap_or(false);
        let mark = if hook_installed {
            term::ok_mark()
        } else {
            term::err_mark()
        };
        let suffix = if hook_installed {
            String::new()
        } else {
            format!("  {}(not installed){}", term::bright_red(), term::reset())
        };
        println!(
            "  {}🎯 Editor hook{}  {}  {}{}",
            term::bold(),
            term::reset(),
            settings_path.display(),
            mark,
            suffix
        );
        println!();
        Ok(())
    }

    /// Remove the NTK hook from editor settings.json.
    pub fn uninstall(&self) -> Result<()> {
        let settings_path = editor_settings_path(self.editor)?;
        if !settings_path.exists() {
            println!(
                "  {} Editor settings not found — nothing to uninstall.",
                term::warn_mark()
            );
            return Ok(());
        }

        let contents = std::fs::read_to_string(&settings_path)
            .with_context(|| format!("reading {}", settings_path.display()))?;

        if !contents.contains(NTK_HOOK_MARKER) {
            println!(
                "  {} NTK hook not found in settings — nothing to uninstall.",
                term::warn_mark()
            );
            return Ok(());
        }

        let sp = term::Spinner::start("Removing NTK hook …");
        let new_contents = remove_ntk_hook_from_json(&contents)?;
        match write_atomic(&settings_path, &new_contents) {
            Ok(()) => sp.finish_ok(&format!("hook removed from {}", settings_path.display())),
            Err(e) => {
                sp.finish_err(&e.to_string());
                return Err(e);
            }
        }

        println!(
            "  {}💾 Config and metrics preserved at ~/.ntk/{}",
            term::dim(),
            term::reset()
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Resolve home directory: `NTK_HOME` env var overrides `dirs::home_dir()`.
/// This allows tests (and advanced users) to point NTK at a custom location.
fn home_dir() -> Result<PathBuf> {
    if let Ok(v) = std::env::var("NTK_HOME") {
        return Ok(PathBuf::from(v));
    }
    dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))
}

fn ntk_bin_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".ntk").join("bin"))
}

fn global_config_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".ntk").join("config.json"))
}

fn editor_settings_path(editor: EditorTarget) -> Result<PathBuf> {
    let home = home_dir()?;
    match editor {
        EditorTarget::ClaudeCode => Ok(home.join(".claude").join("settings.json")),
        EditorTarget::OpenCode => Ok(home.join(".opencode").join("settings.json")),
        EditorTarget::Cursor => Ok(home.join(".cursor").join("mcp.json")),
        EditorTarget::Continue => Ok(home.join(".continue").join("config.json")),
        EditorTarget::Zed => {
            // Zed reads its user settings from:
            //   Linux:   ~/.config/zed/settings.json
            //   macOS:   ~/.config/zed/settings.json (XDG-first, overrides the
            //            Application Support path for dev installs)
            //   Windows: %APPDATA%\Zed\settings.json
            // dirs::config_dir() resolves to the correct root on each OS.
            let cfg =
                dirs::config_dir().ok_or_else(|| anyhow!("cannot determine config directory"))?;
            Ok(cfg.join("zed").join("settings.json"))
        }
    }
}

#[cfg(target_os = "windows")]
fn hook_script_name() -> &'static str {
    "ntk-hook.ps1"
}

#[cfg(not(target_os = "windows"))]
fn hook_script_name() -> &'static str {
    "ntk-hook.sh"
}

#[cfg(target_os = "windows")]
fn ntk_binary_name() -> &'static str {
    "ntk.exe"
}

#[cfg(not(target_os = "windows"))]
fn ntk_binary_name() -> &'static str {
    "ntk"
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("creating directory {}", path.display()))
}

fn print_status_line(label: &str, path: &Path) {
    let (mark, color) = if path.exists() {
        (term::ok_mark(), "")
    } else {
        (term::err_mark(), "")
    };
    let _ = color;
    println!(
        "  {}{}{}  {}  {}",
        term::bold(),
        label,
        term::reset(),
        path.display(),
        mark
    );
}

/// Write file atomically: write to .tmp, then rename (atomic on NTFS + Unix).
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("ntk.tmp");
    std::fs::write(&tmp, content)
        .with_context(|| format!("writing temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))
}

fn copy_hook_script(bin_dir: &Path) -> Result<()> {
    let dest = bin_dir.join(hook_script_name());

    // Embed the hook scripts at compile time.
    #[cfg(target_os = "windows")]
    let content = include_str!("../scripts/ntk-hook.ps1");
    #[cfg(not(target_os = "windows"))]
    let content = include_str!("../scripts/ntk-hook.sh");

    std::fs::write(&dest, content)
        .with_context(|| format!("writing hook script to {}", dest.display()))?;

    // Set executable bit on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("setting permissions on {}", dest.display()))?;
    }

    Ok(())
}

fn create_default_config() -> Result<String> {
    let config_path = global_config_path()?;
    if config_path.exists() {
        return Ok(format!("already exists — {}", config_path.display()));
    }
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("invalid config path"))?;
    ensure_dir(config_dir)?;

    let default_json = serde_json::to_string_pretty(&crate::config::NtkConfig::default())
        .context("serializing default config")?;
    write_atomic(&config_path, &default_json)?;

    // Restrict permissions on Unix: config may contain sensitive paths.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", config_path.display()))?;
    }

    Ok(config_path.display().to_string())
}

// ---------------------------------------------------------------------------
// Binary installation
// ---------------------------------------------------------------------------

/// Copy the currently running NTK binary to `~/.ntk/bin/ntk[.exe]`.
/// Skipped if already running from that location.
fn install_ntk_binary(bin_dir: &Path) -> Result<String> {
    let current_exe = std::env::current_exe().context("getting current executable path")?;
    let dest = bin_dir.join(ntk_binary_name());

    // Canonicalize both paths to avoid copying a file over itself.
    let canon_src = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.clone());
    let canon_dst = dest.canonicalize().unwrap_or_else(|_| dest.clone());
    if canon_src == canon_dst {
        return Ok(format!("already installed — {}", dest.display()));
    }

    std::fs::copy(&current_exe, &dest)
        .with_context(|| format!("copying ntk binary to {}", dest.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("setting permissions on {}", dest.display()))?;
    }

    Ok(dest.display().to_string())
}

// ---------------------------------------------------------------------------
// PATH management
// ---------------------------------------------------------------------------

/// Add `dir` to the persistent user PATH. Idempotent.
fn add_dir_to_path(dir: &Path) -> Result<String> {
    let dir_str = dir.to_string_lossy().into_owned();

    #[cfg(target_os = "windows")]
    {
        windows_add_to_path(&dir_str)
    }

    #[cfg(not(target_os = "windows"))]
    {
        unix_add_to_path(&dir_str)
    }
}

#[cfg(target_os = "windows")]
fn windows_add_to_path(dir: &str) -> Result<String> {
    // Read current user PATH via PowerShell.
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "[Environment]::GetEnvironmentVariable('PATH', 'User')",
        ])
        .output()
        .context("reading user PATH via PowerShell")?;

    let current = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    // Idempotence: skip if already present (case-insensitive on Windows).
    if current
        .split(';')
        .any(|p| p.trim().eq_ignore_ascii_case(dir))
    {
        return Ok(format!("already in PATH — {dir}"));
    }

    let new_path = format!("{current};{dir}");
    let cmd = format!(
        "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
        new_path.replace('\'', "''") // escape single quotes
    );

    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &cmd])
        .status()
        .context("setting user PATH via PowerShell")?;

    if !status.success() {
        return Err(anyhow!("PowerShell failed to update PATH"));
    }

    Ok(format!("added {dir} — open a new terminal to apply"))
}

#[cfg(not(target_os = "windows"))]
fn unix_add_to_path(dir: &str) -> Result<String> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let rc_files = shell_rc_files(&shell);

    let export_line = format!("export PATH=\"$PATH:{dir}\"");

    // Idempotence: check all candidate rc files first.
    for rc in &rc_files {
        if rc.exists() {
            let content =
                std::fs::read_to_string(rc).with_context(|| format!("reading {}", rc.display()))?;
            if content.contains(&export_line) {
                return Ok(format!("already in PATH — {dir}"));
            }
        }
    }

    // Write to the first existing rc file, or ~/.profile as fallback.
    let target = rc_files
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".profile"));

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&target)
        .with_context(|| format!("opening {}", target.display()))?;

    let block = format!("\n# NTK — Neural Token Killer\n{export_line}\n");
    std::io::Write::write_all(&mut file, block.as_bytes())
        .with_context(|| format!("writing to {}", target.display()))?;

    Ok(format!(
        "added to {} — run: source {}",
        target.display(),
        target.display()
    ))
}

#[cfg(not(target_os = "windows"))]
fn shell_rc_files(shell: &str) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    if shell.contains("zsh") {
        vec![home.join(".zshrc"), home.join(".zprofile")]
    } else if shell.contains("fish") {
        let fish_config = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("fish")
            .join("config.fish");
        vec![fish_config]
    } else {
        // bash or unknown
        vec![
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
        ]
    }
}

// ---------------------------------------------------------------------------
// Ollama PATH detection
// ---------------------------------------------------------------------------

/// Detect, install (if missing), and configure Ollama in PATH.
/// Tries silent installation via the platform's package manager or official installer.
pub fn setup_ollama_path() -> Result<String> {
    // Skip installation in test/CI environments.
    if std::env::var("NTK_SKIP_OLLAMA_INSTALL").is_ok() {
        return Err(anyhow!(
            "not found — Layer 3 inference disabled.\n    Install Ollama later: https://ollama.com/download\n    Then run: ntk model pull"
        ));
    }

    // Already in PATH — nothing to do.
    if ollama_in_path() {
        return Ok("already in PATH".to_owned());
    }

    // Installed but not in PATH — just add the dir.
    let candidates = ollama_candidates();
    for candidate in &candidates {
        if candidate.exists() {
            let dir = candidate
                .parent()
                .ok_or_else(|| anyhow!("cannot get parent of {}", candidate.display()))?;
            add_dir_to_path(dir)?;
            return Ok(format!("found at {} — added to PATH", candidate.display()));
        }
    }

    // Not installed — attempt automatic installation.
    match install_ollama() {
        Ok(()) => {
            // After installation, add to PATH (some installers do it, some don't).
            let candidates = ollama_candidates();
            for candidate in &candidates {
                if candidate.exists() {
                    let dir = candidate
                        .parent()
                        .ok_or_else(|| anyhow!("cannot get parent of {}", candidate.display()))?;
                    add_dir_to_path(dir)?;
                    return Ok(format!(
                        "installed at {} — run `ntk model pull`",
                        candidate.display()
                    ));
                }
            }
            Ok("installed — open a new terminal and run `ntk model pull`".to_owned())
        }
        Err(_) => Err(anyhow!(
            "not found — Layer 3 inference disabled.\n    Install Ollama later: https://ollama.com/download\n    Then run: ntk model pull"
        )),
    }
}

/// Attempt to install Ollama using the platform's standard method.
#[cfg(target_os = "windows")]
fn install_ollama() -> Result<()> {
    // Try winget first (available on Windows 10 1709+ / Windows 11).
    let winget = std::process::Command::new("winget")
        .args([
            "install",
            "--id",
            "Ollama.Ollama",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ])
        .status();

    match winget {
        Ok(s) if s.success() => return Ok(()),
        Ok(_) => {}  // winget ran but failed — try direct download
        Err(_) => {} // winget not available — try direct download
    }

    // Fallback: download the official installer and run it silently.
    let installer_url = "https://ollama.com/download/OllamaSetup.exe";
    let tmp = std::env::temp_dir().join("OllamaSetup.exe");

    download_file(installer_url, &tmp)?;

    let status = std::process::Command::new(&tmp)
        .args(["/S"]) // NSIS silent install flag
        .status()
        .context("running OllamaSetup.exe /S")?;

    if !status.success() {
        return Err(anyhow!("Ollama installer exited with error"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_ollama() -> Result<()> {
    // Try Homebrew first (most common on macOS).
    let brew = std::process::Command::new("brew")
        .args(["install", "ollama"])
        .status();

    match brew {
        Ok(s) if s.success() => return Ok(()),
        _ => {} // Homebrew unavailable or failed — try official .zip
    }

    // Fallback: download the official macOS app (.zip) and install to /Applications.
    let url = "https://ollama.com/download/Ollama-darwin.zip";
    let tmp_zip = std::env::temp_dir().join("Ollama-darwin.zip");
    download_file(url, &tmp_zip)?;

    // Unzip to /Applications.
    let status = std::process::Command::new("unzip")
        .args(["-o", &tmp_zip.to_string_lossy(), "-d", "/Applications"])
        .status()
        .context("unzipping Ollama-darwin.zip")?;

    if !status.success() {
        return Err(anyhow!("unzip failed for Ollama-darwin.zip"));
    }

    // The CLI is inside the .app bundle; symlink it to /usr/local/bin.
    let cli = std::path::Path::new("/Applications/Ollama.app/Contents/MacOS/ollama");
    let link = std::path::Path::new("/usr/local/bin/ollama");
    if cli.exists() && !link.exists() {
        std::os::unix::fs::symlink(cli, link).context("symlinking ollama CLI")?;
    }
    Ok(())
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn install_ollama() -> Result<()> {
    // Official Linux install script.
    let status = std::process::Command::new("sh")
        .args(["-c", "curl -fsSL https://ollama.com/install.sh | sh"])
        .status()
        .context("running Ollama install script")?;

    if !status.success() {
        return Err(anyhow!("Ollama install script failed"));
    }
    Ok(())
}

/// Download `url` to `dest` using reqwest (blocking via new runtime).
#[allow(dead_code)]
fn download_file(url: &str, dest: &std::path::Path) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for download")?;

    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // 5 min for large downloads
            .build()
            .context("building HTTP client")?;

        let mut response = client
            .get(url)
            .send()
            .await
            .with_context(|| format!("downloading {url}"))?;

        if !response.status().is_success() {
            return Err(anyhow!("HTTP {} downloading {url}", response.status()));
        }

        let mut file =
            std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;

        while let Some(chunk) = response.chunk().await.context("reading download chunk")? {
            std::io::Write::write_all(&mut file, &chunk)
                .with_context(|| format!("writing to {}", dest.display()))?;
        }
        Ok(())
    })
}

fn ollama_in_path() -> bool {
    let cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    std::process::Command::new(cmd)
        .arg("ollama")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ollama_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        vec![
            PathBuf::from(&local)
                .join("Programs")
                .join("Ollama")
                .join("ollama.exe"),
            PathBuf::from(r"C:\Program Files\Ollama\ollama.exe"),
        ]
    }

    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        vec![
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/opt/homebrew/bin/ollama"),
            home.join(".local").join("bin").join("ollama"),
        ]
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let home = dirs::home_dir().unwrap_or_default();
        vec![
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/usr/bin/ollama"),
            home.join(".local").join("bin").join("ollama"),
        ]
    }
}

// ---------------------------------------------------------------------------
// settings.json patch / unpatch
// ---------------------------------------------------------------------------

/// Inject the NTK PostToolUse hook into editor settings.json.
/// Idempotent: if the hook is already present, does nothing.
fn patch_settings(settings_path: &Path, auto_patch: bool, editor: EditorTarget) -> Result<()> {
    // Ensure parent dir exists.
    if let Some(parent) = settings_path.parent() {
        ensure_dir(parent)?;
    }

    // Load or start with empty JSON object.
    let existing = if settings_path.exists() {
        std::fs::read_to_string(settings_path)
            .with_context(|| format!("reading {}", settings_path.display()))?
    } else {
        "{}".to_owned()
    };

    // Idempotence: entry already present under the editor's marker.
    if existing.contains(NTK_HOOK_MARKER) {
        #[cfg(target_os = "windows")]
        if !editor.uses_mcp() && existing.contains("File ~/.ntk") {
            // Remove stale hook then re-inject with corrected absolute path.
            let cleaned = remove_ntk_hook_from_json(&existing)?;
            let new_json = inject_ntk_hook(&cleaned)?;
            let backup = settings_path.with_extension("ntk.bak");
            if settings_path.exists() && !backup.exists() {
                std::fs::copy(settings_path, &backup)
                    .with_context(|| format!("creating backup {}", backup.display()))?;
            }
            return write_atomic(settings_path, &new_json);
        }
        return Ok(());
    }

    let new_json = match editor {
        EditorTarget::Cursor => inject_ntk_mcp_server(&existing)?,
        EditorTarget::Continue => inject_ntk_continue_mcp_server(&existing)?,
        EditorTarget::Zed => inject_ntk_zed_mcp_server(&existing)?,
        EditorTarget::ClaudeCode | EditorTarget::OpenCode => inject_ntk_hook(&existing)?,
    };

    // In non-auto mode, always proceed (interactive prompt not supported in
    // library code — callers can add prompting around this function).
    let _ = auto_patch;

    // Backup before modifying.
    let backup = settings_path.with_extension("ntk.bak");
    if settings_path.exists() && !backup.exists() {
        std::fs::copy(settings_path, &backup)
            .with_context(|| format!("creating backup {}", backup.display()))?;
    }

    write_atomic(settings_path, &new_json)
}

/// Inject `{"mcpServers": {"ntk": {"command": "ntk", "args": ["mcp-server"]}}}`
/// into a Cursor-style mcp.json without destroying existing entries. This
/// is the MCP equivalent of `inject_ntk_hook` for PostToolUse-style editors.
fn inject_ntk_mcp_server(json_str: &str) -> Result<String> {
    let mut root: serde_json::Value =
        serde_json::from_str(json_str).with_context(|| "parsing mcp.json")?;

    // Resolve the absolute path to the installed `ntk` binary when
    // available; fall back to the bare "ntk" command assuming PATH.
    let ntk_cmd = resolve_ntk_binary_for_mcp();

    let server_entry = serde_json::json!({
        "command": ntk_cmd,
        "args": ["mcp-server"],
        "_ntk": NTK_HOOK_MARKER,
    });

    let servers = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("mcp.json root is not an object"))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("mcp.json[\"mcpServers\"] is not an object"))?;

    servers.insert("ntk".to_string(), server_entry);

    serde_json::to_string_pretty(&root).context("serializing patched mcp.json")
}

/// Inject NTK's MCP server into Continue's `mcpServers` array.
/// Unlike Cursor (object keyed by name), Continue expects an array of
/// objects with an inline `name` field.
fn inject_ntk_continue_mcp_server(json_str: &str) -> Result<String> {
    let mut root: serde_json::Value =
        serde_json::from_str(json_str).with_context(|| "parsing Continue config.json")?;

    let ntk_cmd = resolve_ntk_binary_for_mcp();

    let entry = serde_json::json!({
        "name": "ntk",
        "command": ntk_cmd,
        "args": ["mcp-server"],
        "_ntk": NTK_HOOK_MARKER,
    });

    let servers = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("Continue config.json root is not an object"))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("Continue config.json[\"mcpServers\"] is not an array"))?;

    // Idempotence safety: don't duplicate if a marker-carrying entry exists.
    let already = servers.iter().any(|v| {
        v.get("_ntk")
            .and_then(|m| m.as_str())
            .map(|s| s == NTK_HOOK_MARKER)
            .unwrap_or(false)
    });
    if !already {
        servers.push(entry);
    }

    serde_json::to_string_pretty(&root).context("serializing patched Continue config.json")
}

/// Inject NTK's MCP server into Zed's `context_servers` object. Zed
/// uses a different shape from Cursor: the command lives inside a
/// nested `command` object with `path`, `args`, and `env` keys.
fn inject_ntk_zed_mcp_server(json_str: &str) -> Result<String> {
    let mut root: serde_json::Value =
        serde_json::from_str(json_str).with_context(|| "parsing Zed settings.json")?;

    let ntk_cmd = resolve_ntk_binary_for_mcp();

    let server_entry = serde_json::json!({
        "command": {
            "path": ntk_cmd,
            "args": ["mcp-server"],
            "env": {}
        },
        "_ntk": NTK_HOOK_MARKER,
    });

    let servers = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("Zed settings.json root is not an object"))?
        .entry("context_servers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("Zed settings.json[\"context_servers\"] is not an object"))?;

    servers.insert("ntk".to_string(), server_entry);

    serde_json::to_string_pretty(&root).context("serializing patched Zed settings.json")
}

/// Best-effort absolute path to the installed `ntk` binary — Cursor
/// spawns the MCP server from an environment where PATH may differ
/// from the shell's, so an absolute command is the safe default.
/// Falls back to the bare "ntk" string when the usual install path
/// isn't present.
fn resolve_ntk_binary_for_mcp() -> String {
    if let Ok(home) = home_dir() {
        let bin = home
            .join(".ntk")
            .join("bin")
            .join(if cfg!(windows) { "ntk.exe" } else { "ntk" });
        if bin.exists() {
            return bin.display().to_string();
        }
    }
    "ntk".to_string()
}

/// Inject the NTK hook entry into the JSON string without losing existing content.
fn inject_ntk_hook(json_str: &str) -> Result<String> {
    let mut root: serde_json::Value =
        serde_json::from_str(json_str).with_context(|| "parsing settings.json")?;

    let hook_entry = serde_json::json!({
        "matcher": "Bash",
        "hooks": [{
            "type": "command",
            "command": hook_command()
        }],
        "_ntk": NTK_HOOK_MARKER
    });

    // Ensure root["hooks"]["PostToolUse"] exists as an array.
    let hooks = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings.json root is not an object"))?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings.json[\"hooks\"] is not an object"))?
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("settings.json[\"hooks\"][\"PostToolUse\"] is not an array"))?;

    hooks.push(hook_entry);

    serde_json::to_string_pretty(&root).context("serializing patched settings.json")
}

/// Remove the NTK hook entry from the JSON string.
fn remove_ntk_hook_from_json(json_str: &str) -> Result<String> {
    let mut root: serde_json::Value =
        serde_json::from_str(json_str).with_context(|| "parsing settings.json")?;

    // PostToolUse path (Claude Code / OpenCode): remove any entry carrying
    // the _ntk marker.
    if let Some(post_tool_use) = root
        .pointer_mut("/hooks/PostToolUse")
        .and_then(|v| v.as_array_mut())
    {
        post_tool_use.retain(|entry| {
            entry
                .get("_ntk")
                .and_then(|v| v.as_str())
                .map(|s| s != NTK_HOOK_MARKER)
                .unwrap_or(true)
        });
    }

    // MCP paths: drop our entry from Cursor's mcpServers (object),
    // Continue's mcpServers (array), OR Zed's context_servers (object).
    // Match on the _ntk marker rather than the key/name so uninstall
    // never clobbers a user-renamed server.
    for ptr in &["/mcpServers", "/context_servers"] {
        if let Some(val) = root.pointer_mut(ptr) {
            if let Some(servers) = val.as_object_mut() {
                let keys_to_drop: Vec<String> = servers
                    .iter()
                    .filter(|(_, v)| {
                        v.get("_ntk")
                            .and_then(|m| m.as_str())
                            .map(|s| s == NTK_HOOK_MARKER)
                            .unwrap_or(false)
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in keys_to_drop {
                    servers.remove(&k);
                }
            } else if let Some(arr) = val.as_array_mut() {
                // Continue-style array: retain entries without our marker.
                arr.retain(|v| {
                    v.get("_ntk")
                        .and_then(|m| m.as_str())
                        .map(|s| s != NTK_HOOK_MARKER)
                        .unwrap_or(true)
                });
            }
        }
    }

    serde_json::to_string_pretty(&root).context("serializing cleaned settings.json")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_settings(content: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn test_patch_adds_hook() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::ClaudeCode).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(NTK_HOOK_MARKER));
        assert!(content.contains("PostToolUse"));
    }

    #[test]
    fn test_idempotent_patch() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::ClaudeCode).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        patch_settings(&path, true, EditorTarget::ClaudeCode).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        // Hook must not be duplicated.
        assert_eq!(
            first.matches(NTK_HOOK_MARKER).count(),
            second.matches(NTK_HOOK_MARKER).count()
        );
    }

    #[test]
    fn test_uninstall_removes_hook() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::ClaudeCode).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains(NTK_HOOK_MARKER));

        let content = std::fs::read_to_string(&path).unwrap();
        let cleaned = remove_ntk_hook_from_json(&content).unwrap();
        std::fs::write(&path, &cleaned).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains(NTK_HOOK_MARKER));
    }

    #[test]
    fn test_backup_created_before_patch() {
        let (_dir, path) = temp_settings("{\"existing\": true}");
        patch_settings(&path, true, EditorTarget::ClaudeCode).unwrap();
        let backup = path.with_extension("ntk.bak");
        assert!(backup.exists(), "backup file should be created");
        let bak_content = std::fs::read_to_string(&backup).unwrap();
        assert!(bak_content.contains("\"existing\""));
    }

    #[test]
    fn test_patch_adds_cursor_mcp_server() {
        // Cursor uses mcp.json with mcpServers; make sure the MCP path
        // is taken and the entry carries the _ntk marker for uninstall.
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Cursor).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("mcpServers"),
            "cursor path must write mcpServers: {content}"
        );
        assert!(
            content.contains("mcp-server"),
            "cursor entry must register ntk mcp-server: {content}"
        );
        assert!(
            content.contains(NTK_HOOK_MARKER),
            "_ntk marker missing from cursor entry: {content}"
        );
        // Must NOT write a PostToolUse hook into Cursor's mcp.json.
        assert!(
            !content.contains("PostToolUse"),
            "cursor must not get a PostToolUse hook: {content}"
        );
    }

    #[test]
    fn test_uninstall_removes_cursor_mcp_entry() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Cursor).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let cleaned = remove_ntk_hook_from_json(&content).unwrap();
        assert!(
            !cleaned.contains(NTK_HOOK_MARKER),
            "marker still present: {cleaned}"
        );
    }

    #[test]
    fn test_patch_adds_zed_context_server() {
        // Zed uses context_servers with the nested command object.
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Zed).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("context_servers"),
            "zed path must write context_servers: {content}"
        );
        assert!(
            content.contains("mcp-server"),
            "zed entry must register ntk mcp-server: {content}"
        );
        assert!(
            content.contains(NTK_HOOK_MARKER),
            "_ntk marker missing from zed entry: {content}"
        );
        // Zed's shape nests the command — check for the path field.
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let cmd = v
            .pointer("/context_servers/ntk/command")
            .expect("nested command object");
        assert!(cmd.get("path").is_some(), "zed command missing path: {cmd}");
        assert!(cmd.get("args").is_some(), "zed command missing args: {cmd}");
    }

    #[test]
    fn test_uninstall_removes_zed_context_server() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Zed).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let cleaned = remove_ntk_hook_from_json(&content).unwrap();
        assert!(
            !cleaned.contains(NTK_HOOK_MARKER),
            "marker still present: {cleaned}"
        );
    }

    #[test]
    fn test_patch_adds_continue_mcp_array_entry() {
        // Continue's mcpServers is an ARRAY of server objects keyed by
        // an inline `name` field — distinct from Cursor's object map.
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Continue).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = v["mcpServers"].as_array().expect("mcpServers array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "ntk");
        assert_eq!(arr[0]["_ntk"], NTK_HOOK_MARKER);
        assert_eq!(arr[0]["args"][0], "mcp-server");
    }

    #[test]
    fn test_continue_patch_is_idempotent() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Continue).unwrap();
        patch_settings(&path, true, EditorTarget::Continue).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        // Must not duplicate into two array entries.
        assert_eq!(v["mcpServers"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_uninstall_removes_continue_array_entry() {
        let (_dir, path) = temp_settings("{}");
        patch_settings(&path, true, EditorTarget::Continue).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let cleaned = remove_ntk_hook_from_json(&content).unwrap();
        assert!(
            !cleaned.contains(NTK_HOOK_MARKER),
            "marker still present: {cleaned}"
        );
        // The array itself may remain empty, that's fine; verify the
        // NTK entry specifically is gone.
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        if let Some(arr) = v.get("mcpServers").and_then(|v| v.as_array()) {
            assert!(arr.iter().all(|e| e["name"] != "ntk"));
        }
    }

    #[test]
    fn test_inject_preserves_existing_json() {
        let original = serde_json::json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [{"matcher": "Read"}]
            }
        })
        .to_string();
        let patched = inject_ntk_hook(&original).unwrap();
        let v: serde_json::Value = serde_json::from_str(&patched).unwrap();
        // Original keys preserved.
        assert_eq!(v["theme"], "dark");
        // PreToolUse preserved.
        assert!(v["hooks"]["PreToolUse"].as_array().is_some());
        // NTK hook added.
        assert!(patched.contains(NTK_HOOK_MARKER));
    }
}
