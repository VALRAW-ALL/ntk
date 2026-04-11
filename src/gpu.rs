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
/// CUDA → Metal → Intel AMX → AVX-512 → AVX2 → CPU scalar.
pub fn detect_best_backend() -> GpuBackend {
    if let Some(b) = detect_cuda() {
        return b;
    }
    if detect_metal() {
        return GpuBackend::MetalApple;
    }
    detect_cpu_backend()
}

/// One-line description for use in `ntk status`.
pub fn backend_info(b: &GpuBackend) -> String {
    format!("Inference backend: {b}")
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
