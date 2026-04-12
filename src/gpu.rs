// ---------------------------------------------------------------------------
// Etapa 21 — GPU backend detection
//
// Detects the best available inference backend at startup.
// Pure detection only — no model loading here.
// ---------------------------------------------------------------------------

use std::fmt;

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
    // rocm-smi --showproductname prints lines like:
    //   GPU[0]         : Card series:          Radeon RX 6800 XT
    let out = std::process::Command::new("rocm-smi")
        .args(["--showproductname"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains("card series") || lower.contains("product name") {
            if let Some(val) = line.split(':').next_back() {
                let name = val.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
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

/// Run `nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits`
/// and parse VRAM from the first GPU.
fn run_nvidia_smi() -> Option<GpuBackend> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?.trim();
    let vram_mb: u64 = first_line.parse().ok()?;

    Some(GpuBackend::CudaNvidia {
        device_id: 0,
        vram_mb,
    })
}

// ---------------------------------------------------------------------------
// AMD ROCm detection
// ---------------------------------------------------------------------------

/// Run `rocm-smi --showmeminfo vram --csv` and parse VRAM from the first GPU.
fn run_rocm_smi() -> Option<GpuBackend> {
    let output = std::process::Command::new("rocm-smi")
        .args(["--showmeminfo", "vram", "--csv"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // CSV output format:
    // device,VRAM Total Memory (B),VRAM Total Used Memory (B)
    // card0,17163091968,10485760
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.splitn(3, ',').collect();
        if parts.len() >= 2 {
            if let Ok(bytes) = parts[1].trim().parse::<u64>() {
                return Some(GpuBackend::AmdGpu {
                    device_id: 0,
                    vram_mb: bytes / 1_048_576,
                });
            }
        }
    }
    None
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
