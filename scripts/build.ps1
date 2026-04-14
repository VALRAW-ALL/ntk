#!/usr/bin/env pwsh
# NTK auto-build — detects GPU/OS and invokes `cargo build` with the right
# Cargo feature flag.
#
#   NVIDIA + CUDA Toolkit installed   →  --features cuda
#   Apple Silicon (macOS aarch64)     →  --features metal
#   Everything else (AMD, Intel, ...) →  default features (Candle CPU)
#
# Pass through any extra cargo args (e.g. `./build.ps1 --profile profiling`).

param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $ExtraArgs)

$ErrorActionPreference = 'Stop'

function Test-Cmd($name) {
    $null -ne (Get-Command $name -ErrorAction SilentlyContinue)
}

$feature = $null

# --- CUDA: NVIDIA GPU + nvcc present -----------------------------------------
$hasNvidiaGpu = $false
if (Test-Cmd 'nvidia-smi') {
    $hasNvidiaGpu = (& nvidia-smi --query-gpu=name --format=csv,noheader 2>$null) -ne $null
}
if ($hasNvidiaGpu -and (Test-Cmd 'nvcc')) {
    $feature = 'cuda'
    Write-Host "Detected: NVIDIA GPU + CUDA Toolkit → building with --features cuda" -ForegroundColor Cyan
}

# --- Metal: only Apple Silicon (not relevant on Windows, listed for parity) --
if (-not $feature) {
    if ($IsMacOS -and ($(uname -m) -eq 'arm64')) {
        $feature = 'metal'
        Write-Host "Detected: Apple Silicon → building with --features metal" -ForegroundColor Cyan
    }
}

# --- Default: CPU / external-backend mode ------------------------------------
if (-not $feature) {
    Write-Host "No compatible in-process GPU backend detected — using default build (CPU + Ollama/llama.cpp external)." -ForegroundColor Yellow
    Write-Host "  AMD users: inference runs through llama-server (Vulkan) or Ollama." -ForegroundColor DarkGray
}

$cargoArgs = @('build', '--release')
if ($feature) { $cargoArgs += @('--features', $feature) }
if ($ExtraArgs) { $cargoArgs += $ExtraArgs }

Write-Host "-> cargo $($cargoArgs -join ' ')" -ForegroundColor Green
& cargo @cargoArgs
exit $LASTEXITCODE
