# Skill: ntk-ci-validate

Use this skill before pushing any change to `.github/workflows/release.yml` in the NTK project.

## When to invoke

- After editing `release.yml`
- After any version bump that will trigger a Release workflow
- After changing CUDA-related build steps

## Validation checklist

Run each item and report pass/fail:

### 1. Cargo checks (always)
```bash
cargo fmt --check
cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -W clippy::arithmetic_side_effects -D warnings
cargo build --release
```

### 2. Linux CPU build simulation (Docker)
```bash
export MSYS_NO_PATHCONV=1
docker run --rm -v "$(pwd -W)":/src -w /src rust:1-bookworm bash -ec '
  cargo build --release \
    --target x86_64-unknown-linux-gnu \
    --no-default-features \
    --features sqlite-bundled
'
```

### 3. Workflow YAML checks

Verify in `release.yml`:
- [ ] Linux GPU job uses `container: nvidia/cuda:12.5.1-devel-ubuntu22.04`
- [ ] Bootstrap step has: `build-essential`, `$CUDA/bin >> GITHUB_PATH`, `RUSTFLAGS`, `CUDA_COMPUTE_CAP=80`
- [ ] Windows GPU Jimver sub-packages includes: `nvrtc`, `nvrtc_dev`
- [ ] `ilammy/msvc-dev-cmd@v1` step exists for Windows CUDA
- [ ] Git link.exe removal step exists before Build on Windows CUDA
- [ ] `CUDA_COMPUTE_CAP=80` exported in Windows Configure step
- [ ] All `runs-on:` values are GitHub-hosted runners (not `self-hosted`)

### 4. Check for invalid Jimver package names
Must NOT be in sub-packages: `cuda_runtime`, `libcublas`, `libcurand`

## How to check GitHub Actions results without CLI auth

```bash
# Poll latest runs (unauthenticated, 60 req/hour limit)
curl -s "https://api.github.com/repos/VALRAW-ALL/ntk/actions/runs?branch=master&per_page=3" | \
  powershell.exe -NoProfile -Command '
    $j = [Console]::In.ReadToEnd() | ConvertFrom-Json
    foreach ($r in $j.workflow_runs | Select-Object -First 3) {
      "{0,-30} {1,-12} {2}" -f $r.name, $r.status, $r.conclusion
    }
  '
```

Be mindful of the 60 req/hour rate limit when polling. Poll every 30s max.
