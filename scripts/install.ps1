# NTK system installer — Windows (PowerShell)
#
# Usage:
#   irm https://ntk.valraw.com/install.ps1 | iex
#
# Optional non-interactive override:
#   $env:NTK_INSTALL_PLATFORM = 'nvidia' | 'amd' | 'cpu'
#
# The script:
#   1. Enumerates every discrete GPU on the system (NVIDIA + AMD, any number),
#      reading the display-class driver registry so VRAM is accurate even for
#      8 GB+ cards where Win32_VideoController.AdapterRAM truncates to 4 GB.
#   2. Interactive terminal → shows a numbered list and asks which release
#      variant to install (NVIDIA / AMD / CPU-only). Non-interactive defaults
#      to NVIDIA when an NVIDIA GPU is present, otherwise CPU.
#   3. Downloads the matching artifact (`ntk-windows-x86_64-{cpu,gpu}.exe`)
#      from the latest GitHub release.

$ErrorActionPreference = 'Stop'

$Repo = 'VALRAW-ALL/ntk'

# ---------------------------------------------------------------------------
# GPU enumeration
# ---------------------------------------------------------------------------

function Get-GpuList {
    # Returns an array of [pscustomobject]@{ Vendor; Name; VramMb }
    $gpus = @()
    try {
        $base = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
        Get-ChildItem $base -ErrorAction SilentlyContinue | ForEach-Object {
            $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
            if (-not $p) { return }
            if (-not $p.DriverDesc) { return }
            if (-not $p.MatchingDeviceId) { return }

            $vendor = $null
            if ($p.MatchingDeviceId -match 'VEN_10DE') { $vendor = 'NVIDIA' }
            elseif ($p.MatchingDeviceId -match 'VEN_1002') { $vendor = 'AMD' }
            else { return }

            $mb = 0
            $qw = $p.'HardwareInformation.qwMemorySize'
            $sz = $p.'HardwareInformation.MemorySize'
            if ($qw) { $mb = [int64]([math]::Round($qw / 1MB)) }
            elseif ($sz) { $mb = [int64]([math]::Round(($sz -as [int64]) / 1MB)) }

            $gpus += [pscustomobject]@{
                Vendor = $vendor
                Name   = $p.DriverDesc
                VramMb = $mb
            }
        }
    } catch {
        # Registry unreachable — return what we have so far.
    }
    return $gpus
}

$gpus    = Get-GpuList
$nvidia  = @($gpus | Where-Object { $_.Vendor -eq 'NVIDIA' })
$amd     = @($gpus | Where-Object { $_.Vendor -eq 'AMD' })
$hasNv   = $nvidia.Count -gt 0
$hasAmd  = $amd.Count -gt 0

# ---------------------------------------------------------------------------
# Platform selection
# ---------------------------------------------------------------------------

Write-Host ''
Write-Host '  NTK installer'
Write-Host '  ─────────────'
Write-Host ''

if ($hasNv -or $hasAmd) {
    Write-Host '  Detected GPUs:'
    $i = 1
    foreach ($g in $nvidia + $amd) {
        $vram = if ($g.VramMb -gt 0) { "  ($($g.VramMb) MB VRAM)" } else { '' }
        Write-Host ("    GPU #{0}  {1,-7} {2}{3}" -f $i, $g.Vendor, $g.Name, $vram)
        $i++
    }
    Write-Host ''
}

# Default = NVIDIA if detected, else CPU. AMD is never auto-default — it
# still installs the CPU binary and prints llama-server Vulkan instructions.
$default = if ($hasNv) { 'nvidia' } else { 'cpu' }
$platform = $env:NTK_INSTALL_PLATFORM

if (-not $platform) {
    $interactive = [Environment]::UserInteractive `
                   -and $Host.UI.RawUI `
                   -and -not [Console]::IsInputRedirected

    if ($interactive) {
        Write-Host '  Which release do you want to install?'
        Write-Host '    [1] NVIDIA (GPU build, CUDA)'
        Write-Host '    [2] AMD    (CPU build + configure llama-server Vulkan later)'
        Write-Host '    [3] CPU only'
        Write-Host ''
        $defNum = switch ($default) { 'nvidia' { 1 } 'amd' { 2 } default { 3 } }
        $choice = Read-Host ("  Choose [1/2/3] or Enter for [{0}]" -f $defNum)
        if ([string]::IsNullOrWhiteSpace($choice)) { $choice = "$defNum" }
        switch ($choice) {
            '1' { $platform = 'nvidia' }
            '2' { $platform = 'amd' }
            '3' { $platform = 'cpu' }
            default { throw "Invalid choice: $choice" }
        }
    } else {
        $platform = $default
        Write-Host "  Non-interactive install — selecting '$platform' automatically."
        Write-Host "  (Set `$env:NTK_INSTALL_PLATFORM = 'nvidia' | 'amd' | 'cpu' to override.)"
    }
}

switch ($platform) {
    'nvidia' { $suffix = 'gpu'; $postNote = $null }
    'amd'    { $suffix = 'cpu'; $postNote = 'AMD' }
    'cpu'    { $suffix = 'cpu'; $postNote = $null }
    default  { throw "Invalid NTK_INSTALL_PLATFORM='$platform' (expected nvidia|amd|cpu)" }
}

# ---------------------------------------------------------------------------
# Download + install
# ---------------------------------------------------------------------------

$latest   = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
$artifact = "ntk-windows-x86_64-$suffix.exe"
$url      = "https://github.com/$Repo/releases/download/$latest/$artifact"
$dest     = "$env:LOCALAPPDATA\ntk\ntk.exe"

Write-Host ''
Write-Host "  Downloading $artifact ($latest)…"
New-Item -ItemType Directory -Force -Path "$env:LOCALAPPDATA\ntk" | Out-Null
try {
    Invoke-WebRequest $url -OutFile $dest
} catch {
    Write-Host "Download failed." -ForegroundColor Red
    Write-Host "  URL: $url"
    Write-Host "  This variant may not exist — try another choice." -ForegroundColor Red
    throw
}

# Add to user PATH if not already present.
$currentPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($currentPath -notlike "*$env:LOCALAPPDATA\ntk*") {
    [Environment]::SetEnvironmentVariable(
        'PATH',
        "$currentPath;$env:LOCALAPPDATA\ntk",
        'User'
    )
    Write-Host "  Added $env:LOCALAPPDATA\ntk to PATH."
}

Write-Host ''
Write-Host "  NTK installed to $dest"
Write-Host ''

if ($postNote -eq 'AMD') {
    Write-Host '  Next steps for AMD GPU inference:'
    Write-Host '    1. Install llama.cpp built with Vulkan:'
    Write-Host '         https://github.com/ggerganov/llama.cpp/releases'
    Write-Host '       Place llama-server.exe on your PATH (or %USERPROFILE%\.ntk\bin\).'
    Write-Host "    2. Run: ntk model setup   → choose 'llama.cpp' backend."
    Write-Host ''
}

Write-Host '  Next: ntk init -g'
