// ---------------------------------------------------------------------------
// Etapa 21 — GPU backend detection
//
// Detects the best available inference backend at startup.
// Pure detection only — no model loading here.
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// GpuDevice — one enumerated GPU on the system
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
}

impl GpuVendor {
    pub fn label(&self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Amd => "AMD",
            GpuVendor::Intel => "Intel",
            GpuVendor::Apple => "Apple",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDevice {
    pub vendor: GpuVendor,
    /// Zero-based index within the vendor's enumeration.
    pub device_id: u32,
    pub name: String,
    pub vram_mb: u64,
}

impl fmt::Display for GpuDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.vram_mb > 0 {
            write!(
                f,
                "{} {} ({} MB VRAM)",
                self.vendor.label(),
                self.name,
                self.vram_mb
            )
        } else {
            write!(f, "{} {}", self.vendor.label(), self.name)
        }
    }
}

/// Enumerate every discrete GPU on the system across every supported vendor.
///
/// Used by the model-setup wizard and `ntk start --gpu` to offer a selection
/// list when more than one GPU is present. Detection sources:
///   - NVIDIA: `nvidia-smi` (multi-GPU aware)
///   - AMD:    `rocm-smi` → Windows registry → Linux sysfs (fallback chain)
///   - Apple:  compile-time target (`aarch64-apple-darwin`)
pub fn enumerate_gpus() -> Vec<GpuDevice> {
    let mut out: Vec<GpuDevice> = Vec::new();
    out.extend(nvidia_gpus());
    out.extend(amd_gpus());
    if detect_metal() {
        out.push(GpuDevice {
            vendor: GpuVendor::Apple,
            device_id: 0,
            name: "Apple Silicon GPU".to_string(),
            vram_mb: 0, // unified memory — not applicable
        });
    }
    out
}

// ---------------------------------------------------------------------------
// GpuBackend enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum GpuBackend {
    /// NVIDIA GPU via CUDA.
    CudaNvidia { device_id: u32, vram_mb: u64 },
    /// AMD GPU via ROCm.
    AmdGpu { device_id: u32, vram_mb: u64 },
    /// Apple Silicon GPU via Metal.
    MetalApple,
    /// Intel Advanced Matrix Extensions (Xeon 4th Gen / Core Ultra).
    IntelAmx,
    /// AVX-512 capable x86 CPU.
    Avx512,
    /// AVX2 capable x86 CPU (typical modern desktop/laptop).
    Avx2,
    /// Scalar CPU — most basic fallback.
    CpuScalar,
}

impl fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuBackend::CudaNvidia { device_id, vram_mb } => {
                write!(f, "CUDA (device {device_id}, {vram_mb} MB VRAM)")
            }
            GpuBackend::AmdGpu { device_id, vram_mb } => {
                write!(f, "AMD ROCm (device {device_id}, {vram_mb} MB VRAM)")
            }
            GpuBackend::MetalApple => write!(f, "Metal (Apple Silicon)"),
            GpuBackend::IntelAmx => write!(f, "Intel AMX"),
            GpuBackend::Avx512 => write!(f, "CPU AVX-512"),
            GpuBackend::Avx2 => write!(f, "CPU AVX2"),
            GpuBackend::CpuScalar => write!(f, "CPU Scalar"),
        }
    }
}

// ---------------------------------------------------------------------------
// detect_best_backend
// ---------------------------------------------------------------------------

/// Detect the best available inference backend in priority order:
/// CUDA → AMD ROCm → Metal → Intel AMX → AVX-512 → AVX2 → CPU scalar.
///
/// Used when the user has NOT made an explicit choice via `ntk model setup`.
/// When a choice exists in `config.model.gpu_vendor`, use
/// [`resolve_configured_backend`] instead — it honours the user's selection
/// verbatim and never silently prefers one vendor over another.
pub fn detect_best_backend() -> GpuBackend {
    if let Some(b) = detect_cuda() {
        return b;
    }
    if let Some(b) = detect_amd() {
        return b;
    }
    if detect_metal() {
        return GpuBackend::MetalApple;
    }
    detect_cpu_backend()
}

/// Resolve the active inference backend, honouring the user's explicit choice
/// from `ntk model setup` over automatic detection.
///
/// Rules:
/// - `gpu_layers == 0` → CPU regardless of any other field (user picked CPU).
/// - `gpu_vendor = Some(v)` → locate the matching GPU in the live enumeration
///   at `device_id`. If that specific device is gone, fall back to any device
///   from the same vendor, then finally to CPU. Never silently switch vendor.
/// - `gpu_vendor = None` and `gpu_layers != 0` → auto-detect (legacy path).
pub fn resolve_configured_backend(
    gpu_layers: i32,
    gpu_vendor: Option<GpuVendor>,
    device_id: u32,
) -> GpuBackend {
    if gpu_layers == 0 {
        return detect_cpu_backend();
    }
    let Some(vendor) = gpu_vendor else {
        return detect_best_backend();
    };

    let gpus = enumerate_gpus();
    let pick = gpus
        .iter()
        .find(|g| g.vendor == vendor && g.device_id == device_id)
        .or_else(|| gpus.iter().find(|g| g.vendor == vendor));

    match (vendor, pick) {
        (GpuVendor::Nvidia, Some(g)) => GpuBackend::CudaNvidia {
            device_id: g.device_id,
            vram_mb: g.vram_mb,
        },
        (GpuVendor::Amd, Some(g)) => GpuBackend::AmdGpu {
            device_id: g.device_id,
            vram_mb: g.vram_mb,
        },
        (GpuVendor::Apple, _) if detect_metal() => GpuBackend::MetalApple,
        // Configured vendor is absent — degrade to CPU rather than cross-wire
        // to a different vendor silently.
        _ => detect_cpu_backend(),
    }
}

/// Check for NVIDIA GPU via nvidia-smi. Returns the backend if found.
pub fn detect_nvidia() -> Option<GpuBackend> {
    detect_cuda()
}

/// Check for AMD GPU via rocm-smi. Returns the backend if found.
pub fn detect_amd() -> Option<GpuBackend> {
    run_rocm_smi()
}

/// Returns true if running on Apple Silicon (Metal available).
pub fn is_metal_available() -> bool {
    detect_metal()
}

/// One-line description for use in `ntk status`.
pub fn backend_info(b: &GpuBackend) -> String {
    format!("Inference backend: {b}")
}

/// Returns the GPU model name string (NVIDIA or AMD), if a discrete GPU is detected.
pub fn gpu_model_name() -> Option<String> {
    if let Some(name) = nvidia_name() {
        return Some(name);
    }
    amd_name()
}

/// Returns the CPU model name string (platform-specific).
/// Falls back to an empty string if not determinable.
pub fn cpu_model_name() -> Option<String> {
    cpu_name_impl()
}

/// Short capability label for the current CPU (e.g. "AVX2").
pub fn cpu_capability_label() -> &'static str {
    match detect_cpu_backend() {
        GpuBackend::IntelAmx => "AMX",
        GpuBackend::Avx512 => "AVX-512",
        GpuBackend::Avx2 => "AVX2",
        _ => "Scalar",
    }
}

fn nvidia_name() -> Option<String> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn amd_name() -> Option<String> {
    // Prefer the first AMD device found by the multi-source enumerator,
    // so Polaris/Vega cards on Windows (no ROCm) still report a name.
    amd_gpus().into_iter().next().map(|d| d.name)
}

#[cfg(windows)]
fn cpu_name_impl() -> Option<String> {
    // wmic was deprecated in Windows 11; use PowerShell CIM instead.
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_Processor).Name",
        ])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(target_os = "linux")]
fn cpu_name_impl() -> Option<String> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in cpuinfo.lines() {
        if line.starts_with("model name") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn cpu_name_impl() -> Option<String> {
    let out = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn cpu_name_impl() -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// CUDA detection
// ---------------------------------------------------------------------------

fn detect_cuda() -> Option<GpuBackend> {
    // Skip entirely on non-x86/non-cuda targets at compile time.
    #[cfg(not(feature = "cuda"))]
    {
        // Even without the cuda feature, we can check if nvidia-smi is present
        // and report the device.  The daemon will still use Ollama (which can
        // talk to a GPU via its own runtime), so this is informational.
        if let Some(info) = run_nvidia_smi() {
            return Some(info);
        }
        None
    }

    #[cfg(feature = "cuda")]
    {
        run_nvidia_smi()
    }
}

/// Query `nvidia-smi` for every CUDA device on the system, falling back to
/// the Windows driver registry when `nvidia-smi` is missing (common on
/// machines with the driver installed but no CUDA Toolkit).
fn nvidia_gpus() -> Vec<GpuDevice> {
    let smi = nvidia_gpus_from_smi();
    if !smi.is_empty() {
        return smi;
    }
    #[cfg(windows)]
    {
        windows_gpus_from_registry(GpuVendor::Nvidia)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn nvidia_gpus_from_smi() -> Vec<GpuDevice> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(output) = output else { return Vec::new() };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, ',').map(str::trim).collect();
            if parts.len() < 3 {
                return None;
            }
            let device_id: u32 = parts[0].parse().ok()?;
            let name = parts[1].to_string();
            let vram_mb: u64 = parts[2].parse().ok()?;
            Some(GpuDevice {
                vendor: GpuVendor::Nvidia,
                device_id,
                name,
                vram_mb,
            })
        })
        .collect()
}

/// Back-compat: first NVIDIA device as a `GpuBackend`, or None.
fn run_nvidia_smi() -> Option<GpuBackend> {
    let d = nvidia_gpus().into_iter().next()?;
    Some(GpuBackend::CudaNvidia {
        device_id: d.device_id,
        vram_mb: d.vram_mb,
    })
}

// ---------------------------------------------------------------------------
// AMD detection — tries rocm-smi first, then OS-specific fallbacks so that
// non-ROCm cards (e.g. Polaris RX 570/580/590 on Windows) are still found.
// ---------------------------------------------------------------------------

/// Full list of AMD GPUs on the system. Prefers `rocm-smi` (ROCm-enabled
/// systems); falls back to the Windows driver registry or Linux sysfs when
/// `rocm-smi` is unavailable (which is the common case for Polaris / Vega
/// cards and any Windows host).
fn amd_gpus() -> Vec<GpuDevice> {
    let rocm = amd_gpus_rocm();
    if !rocm.is_empty() {
        return rocm;
    }
    amd_gpus_os_fallback()
}

fn amd_gpus_rocm() -> Vec<GpuDevice> {
    let out = std::process::Command::new("rocm-smi")
        .args(["--showmeminfo", "vram", "--csv"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    // CSV: device,VRAM Total Memory (B),VRAM Total Used Memory (B)
    let stdout = String::from_utf8_lossy(&out.stdout);
    let names = amd_rocm_names();
    stdout
        .lines()
        .skip(1)
        .enumerate()
        .filter_map(|(idx, line)| {
            let parts: Vec<&str> = line.splitn(3, ',').collect();
            if parts.len() < 2 {
                return None;
            }
            let bytes: u64 = parts[1].trim().parse().ok()?;
            let device_id = idx as u32;
            let name = names
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("AMD GPU {device_id}"));
            Some(GpuDevice {
                vendor: GpuVendor::Amd,
                device_id,
                name,
                vram_mb: bytes / 1_048_576,
            })
        })
        .collect()
}

/// Parse `rocm-smi --showproductname` into a per-device name list.
fn amd_rocm_names() -> Vec<String> {
    let out = std::process::Command::new("rocm-smi")
        .args(["--showproductname"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains("card series") || lower.contains("product name") {
            if let Some(val) = line.split(':').next_back() {
                let n = val.trim().to_string();
                if !n.is_empty() {
                    names.push(n);
                }
            }
        }
    }
    names
}

#[cfg(windows)]
fn amd_gpus_os_fallback() -> Vec<GpuDevice> {
    windows_gpus_from_registry(GpuVendor::Amd)
}

/// Enumerate every GPU of the given vendor on Windows via the display-class
/// driver registry.  Works for every installed card regardless of vendor SDKs
/// (ROCm, CUDA Toolkit) being present, and returns accurate 64-bit VRAM via
/// `HardwareInformation.qwMemorySize`.
///
/// `Win32_VideoController.AdapterRAM` is NOT used here: its 32-bit DWORD
/// caps at ~4 GB, which is wrong for 8 GB cards like the RX 580 8GB.
#[cfg(windows)]
fn windows_gpus_from_registry(vendor: GpuVendor) -> Vec<GpuDevice> {
    let vendor_id = match vendor {
        GpuVendor::Amd => "VEN_1002",
        GpuVendor::Nvidia => "VEN_10DE",
        GpuVendor::Intel => "VEN_8086",
        GpuVendor::Apple => return Vec::new(),
    };
    // The script always finishes with `exit 0`: PowerShell otherwise bubbles
    // up a non-zero status when one of the subkeys it iterates is missing a
    // `DriverDesc` property, which would make Rust discard the valid output.
    let script = format!(
        r#"
$ErrorActionPreference = 'SilentlyContinue'
$base = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{{4d36e968-e325-11ce-bfc1-08002be10318}}'
Get-ChildItem $base | ForEach-Object {{
    $p = Get-ItemProperty $_.PSPath
    if ($p -and $p.DriverDesc -and $p.MatchingDeviceId -match '{vendor_id}') {{
        $mb = 0
        if ($p.PSObject.Properties['HardwareInformation.qwMemorySize']) {{
            $mb = [int64]([math]::Round($p.'HardwareInformation.qwMemorySize' / 1MB))
        }} elseif ($p.PSObject.Properties['HardwareInformation.MemorySize']) {{
            $mb = [int64]([math]::Round(($p.'HardwareInformation.MemorySize' -as [int64]) / 1MB))
        }}
        "{{0}}|{{1}}" -f $p.DriverDesc, $mb
    }}
}}
exit 0
"#
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output();
    let Ok(out) = out else { return Vec::new() };
    // NOTE: exit status is intentionally ignored — PowerShell may still return
    // non-zero when individual subkey accesses fail even though the valuable
    // lines are already on stdout.
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.contains('|'))
        .enumerate()
        .filter_map(|(idx, line)| {
            let mut parts = line.splitn(2, '|');
            let name = parts.next()?.trim().to_string();
            let vram_mb: u64 = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
            Some(GpuDevice {
                vendor,
                device_id: idx as u32,
                name,
                vram_mb,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn amd_gpus_os_fallback() -> Vec<GpuDevice> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    let mut idx: u32 = 0;
    let mut seen: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        // Only primary card nodes: cardN (not cardN-HDMI-A-1 connector nodes).
        if !fname_str.starts_with("card") || fname_str.contains('-') {
            continue;
        }
        let device_dir = entry.path().join("device");
        let vendor = std::fs::read_to_string(device_dir.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if vendor != "0x1002" {
            continue;
        }
        // Resolve to canonical PCI path to avoid counting the same GPU twice
        // when multiple `cardN` symlinks point at it.
        let canonical = std::fs::canonicalize(&device_dir).unwrap_or(device_dir.clone());
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical);

        let device_pci = std::fs::read_to_string(device_dir.join("device"))
            .unwrap_or_default()
            .trim()
            .trim_start_matches("0x")
            .to_string();
        let vram_bytes: u64 = std::fs::read_to_string(device_dir.join("mem_info_vram_total"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let name = amd_name_from_lspci(&device_pci).unwrap_or_else(|| "AMD GPU".to_string());
        out.push(GpuDevice {
            vendor: GpuVendor::Amd,
            device_id: idx,
            name,
            vram_mb: vram_bytes / 1_048_576,
        });
        idx += 1;
    }
    out
}

#[cfg(target_os = "linux")]
fn amd_name_from_lspci(device_pci: &str) -> Option<String> {
    let out = std::process::Command::new("lspci")
        .args(["-nn", "-d", &format!("1002:{device_pci}")])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next()?;
    // Example: "03:00.0 VGA ... : Advanced Micro Devices [AMD/ATI] Ellesmere [Radeon RX 570/580/590] [1002:67df]"
    let after = line.splitn(3, ':').nth(2)?.trim();
    // Strip trailing "[1002:xxxx]" vendor/device id tag for readability.
    let cleaned = after
        .rsplit_once(" [")
        .map(|(lhs, _)| lhs.trim().to_string())
        .unwrap_or_else(|| after.to_string());
    Some(cleaned)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn amd_gpus_os_fallback() -> Vec<GpuDevice> {
    Vec::new()
}

/// Back-compat: first AMD device as a `GpuBackend`, or None.
fn run_rocm_smi() -> Option<GpuBackend> {
    let d = amd_gpus().into_iter().next()?;
    Some(GpuBackend::AmdGpu {
        device_id: d.device_id,
        vram_mb: d.vram_mb,
    })
}

// ---------------------------------------------------------------------------
// Metal detection (Apple Silicon)
// ---------------------------------------------------------------------------

fn detect_metal() -> bool {
    // Metal is only available on aarch64-apple-darwin.
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

// ---------------------------------------------------------------------------
// CPU capability detection (AMX / AVX-512 / AVX2)
// ---------------------------------------------------------------------------

fn detect_cpu_backend() -> GpuBackend {
    // Check /proc/cpuinfo on Linux for amx_bf16 / avx512f / avx2 flags.
    #[cfg(target_os = "linux")]
    if let Some(b) = detect_cpu_linux() {
        return b;
    }

    // On x86/x86_64 we can use CPUID at runtime.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        return detect_cpu_x86();
    }

    #[allow(unreachable_code)]
    GpuBackend::CpuScalar
}

#[cfg(target_os = "linux")]
fn detect_cpu_linux() -> Option<GpuBackend> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in cpuinfo.lines() {
        if line.starts_with("flags") {
            if line.contains("amx_bf16") {
                return Some(GpuBackend::IntelAmx);
            }
            if line.contains("avx512f") {
                return Some(GpuBackend::Avx512);
            }
            if line.contains("avx2") {
                return Some(GpuBackend::Avx2);
            }
            break;
        }
    }
    None
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_cpu_x86() -> GpuBackend {
    // is_x86_feature_detected! is a stable std macro on x86/x86_64.
    if is_x86_feature_detected!("avx512f") {
        GpuBackend::Avx512
    } else if is_x86_feature_detected!("avx2") {
        GpuBackend::Avx2
    } else {
        GpuBackend::CpuScalar
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_best_backend_returns_something() {
        // Must always return a valid backend without panicking.
        let backend = detect_best_backend();
        let info = backend_info(&backend);
        assert!(!info.is_empty());
    }

    #[test]
    fn test_backend_display() {
        let b = GpuBackend::CudaNvidia {
            device_id: 0,
            vram_mb: 8192,
        };
        assert!(b.to_string().contains("CUDA"));
        assert!(b.to_string().contains("8192"));

        let amd = GpuBackend::AmdGpu {
            device_id: 0,
            vram_mb: 16384,
        };
        assert!(amd.to_string().contains("AMD"));
        assert!(amd.to_string().contains("16384"));

        assert!(GpuBackend::MetalApple.to_string().contains("Metal"));
        assert!(GpuBackend::Avx2.to_string().contains("AVX2"));
        assert!(GpuBackend::CpuScalar.to_string().contains("Scalar"));
    }

    #[test]
    fn test_detect_metal_compile_time() {
        // On non-Apple targets this must be false.
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert!(!detect_metal());
    }

    #[test]
    fn test_backend_info_format() {
        let info = backend_info(&GpuBackend::Avx2);
        assert!(info.starts_with("Inference backend:"));
    }
}
