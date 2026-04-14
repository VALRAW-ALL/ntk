# Rule: NTK CUDA CI Requirements

Applies to: any change to `.github/workflows/release.yml` in the NTK project that involves CUDA builds.

## Required checklist before committing a CUDA CI change

### Linux GPU job (uses `nvidia/cuda:12.5.1-devel-ubuntu22.04` container)

- [ ] Bootstrap step installs `build-essential` (provides g++ — nvcc needs it as host compiler)
- [ ] `$CUDA/bin` is added to `$GITHUB_PATH` (makes nvcc + nvidia-smi discoverable by build scripts)
- [ ] `LIBRARY_PATH=$CUDA/lib64/stubs` exported (cc linker finds libcuda.so stub)
- [ ] `RUSTFLAGS=-L $CUDA/lib64/stubs` exported (rustc linker finds libcuda.so stub — does NOT inherit LIBRARY_PATH)
- [ ] `CUDA_COMPUTE_CAP=80` exported — candle-core calls nvidia-smi to detect GPU arch; without a physical GPU it fails. This var skips the call and compiles for sm_80 (Ampere+)

### Windows GPU job (uses Jimver/cuda-toolkit + windows-latest)

- [ ] Jimver sub-packages includes ALL of: `nvcc`, `cudart`, `cublas`, `cublas_dev`, `curand`, `curand_dev`, `nvrtc`, `nvrtc_dev`, `visual_studio_integration`
  - `nvrtc` / `nvrtc_dev` are required by cudarc — missing them causes `LNK1181: cannot open input file 'nvrtc.lib'`
  - `visual_studio_integration` registers MSVC with nvcc so it can find cl.exe
- [ ] `ilammy/msvc-dev-cmd@v1` runs BEFORE the Build step — activates vcvarsall.bat so cl.exe is in PATH for nvcc
- [ ] Git's link.exe is removed AFTER ilammy and BEFORE Build:
  ```powershell
  Remove-Item "C:\Program Files\Git\usr\bin\link.exe" -Force -ErrorAction SilentlyContinue
  ```
  Without this, `link.exe` resolves to Git's POSIX hard-link tool instead of the MSVC linker
- [ ] `CUDA_COMPUTE_CAP=80` exported in the Configure CUDA env step

## Known sub-package names (Jimver for Windows)

Valid names for Jimver `sub-packages` JSON array (CUDA 12.5):
`nvcc`, `cudart`, `cublas`, `cublas_dev`, `curand`, `curand_dev`, `nvrtc`, `nvrtc_dev`, `visual_studio_integration`

**INVALID names** (caused apt exit 100 / installer rejection in past iterations):
- `cuda_runtime` — not a real package name (the runtime is `cudart`)
- `libcublas`, `libcurand` — Linux apt names, not Jimver component names

## Linux GPU runner pinning

Pin to `ubuntu-22.04` if NOT using the container approach — CUDA 12.5 apt channel is only available on Jammy (22.04), not Noble (24.04 = ubuntu-latest).

When using the `nvidia/cuda:12.5.1-devel-ubuntu22.04` container, the runner can be `ubuntu-latest` (it's just the Docker host).
