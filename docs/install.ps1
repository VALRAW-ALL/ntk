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

# Single source of truth: ~/.ntk/bin/ntk.exe (same path that `ntk init -g` writes to).
$canonicalDir = Join-Path $env:USERPROFILE '.ntk\bin'
$dest         = Join-Path $canonicalDir 'ntk.exe'

# Show current version if already installed
$current = ''
try { $current = (& ntk --version 2>$null).Split(' ')[1] } catch {}

Write-Host ''
if ($current) {
    Write-Host "  Updating NTK  $current  ->  $latest"
} else {
    Write-Host "  Installing NTK $latest"
}
Write-Host "  Downloading $artifact..."

# Download to a throwaway temp path. We invoke that exe to run `init -g`,
# which is what actually installs the binary into ~/.ntk/bin/. This keeps
# a single canonical path on disk and a single PATH entry.
$tmp = Join-Path $env:TEMP "ntk_installer_$([guid]::NewGuid().Guid.Substring(0,8)).exe"
try {
    Invoke-WebRequest $url -OutFile $tmp
} catch {
    Write-Host 'Download failed.' -ForegroundColor Red
    Write-Host "  URL: $url"
    Write-Host '  This variant may not exist — try another choice.' -ForegroundColor Red
    throw
}

# ---------------------------------------------------------------------------
# Cleanup — remove stale NTK installs and PATH entries from previous runs
# ---------------------------------------------------------------------------
# Past installer versions left ntk.exe copies in $env:TEMP and in
# %LOCALAPPDATA%\ntk, and added each of those dirs to the user/machine PATH,
# accumulating dozens of stale entries over time. Sweep them out before
# `ntk init -g` registers the single canonical entry.

function Test-IsNtkPathEntry {
    param([string]$Entry)
    if ([string]::IsNullOrWhiteSpace($Entry)) { return $false }
    $norm = $Entry.TrimEnd('\').ToLowerInvariant()
    if ($norm -eq $canonicalDir.ToLowerInvariant()) { return $false }  # canonical, keep
    # Catches \Temp\.tmpXXX\.ntk\bin and any other dir that mentions ntk under temp.
    if ($norm -match '\\(temp|tmp)\\.*ntk') { return $true }
    # Plain \ntk or \ntk\bin tail (e.g. old %LOCALAPPDATA%\ntk install dir).
    if ($norm -match '\\ntk(\\bin)?$') { return $true }
    return $false
}

function Remove-StalePathEntries {
    param([ValidateSet('User','Machine')][string]$Scope)

    $current = [Environment]::GetEnvironmentVariable('PATH', $Scope)
    if (-not $current) { return @() }
    $entries = $current.Split(';')
    $stale   = @($entries | Where-Object { Test-IsNtkPathEntry $_ })
    $kept    = @($entries | Where-Object { -not (Test-IsNtkPathEntry $_) -and -not [string]::IsNullOrWhiteSpace($_) })

    if ($stale.Count -eq 0) { return @() }

    $newPath = ($kept -join ';')
    try {
        [Environment]::SetEnvironmentVariable('PATH', $newPath, $Scope)
        Write-Host "  Cleaned $($stale.Count) stale NTK entries from $Scope PATH."
    } catch {
        Write-Host "  Could not update $Scope PATH ($($_.Exception.Message))." -ForegroundColor Yellow
        if ($Scope -eq 'Machine') {
            Write-Host "  Re-run installer in an elevated PowerShell to clean Machine PATH." -ForegroundColor Yellow
        }
        return @()
    }
    return $stale
}

$staleAll = @()
$staleAll += Remove-StalePathEntries -Scope 'User'
$staleAll += Remove-StalePathEntries -Scope 'Machine'

# Delete the on-disk leftovers for every stale entry we just dropped.
foreach ($stale in $staleAll) {
    $dir = $stale.TrimEnd('\')
    if (Test-Path $dir) {
        try {
            Remove-Item -Recurse -Force -ErrorAction Stop $dir
            Write-Host "  Removed stale install: $dir"
        } catch {
            Write-Host "  Could not remove $dir ($($_.Exception.Message))" -ForegroundColor Yellow
        }
    }
}

# Sweep orphaned \Temp\.tmp*\.ntk\ dirs even if PATH no longer references them.
Get-ChildItem "$env:LOCALAPPDATA\Temp" -Directory -Filter '.tmp*' -ErrorAction SilentlyContinue |
    Where-Object { Test-Path "$($_.FullName)\.ntk" } |
    ForEach-Object {
        try { Remove-Item -Recurse -Force -ErrorAction Stop $_.FullName }
        catch { Write-Host "  Could not remove $($_.FullName) ($($_.Exception.Message))" -ForegroundColor Yellow }
    }

# Wipe the obsolete %LOCALAPPDATA%\ntk install dir if the previous installer left it behind.
$legacyDir = Join-Path $env:LOCALAPPDATA 'ntk'
if (Test-Path $legacyDir) {
    try {
        Remove-Item -Recurse -Force -ErrorAction Stop $legacyDir
        Write-Host "  Removed legacy install: $legacyDir"
    } catch {
        Write-Host "  Could not remove $legacyDir ($($_.Exception.Message))" -ForegroundColor Yellow
    }
}

# Refresh current session PATH so the canonical dir resolves once `ntk init` adds it persistently.
$env:PATH = "$env:PATH;$canonicalDir"

# ---------------------------------------------------------------------------
# Step 1 — ntk init -g  (installs binary to ~/.ntk/bin and registers hook)
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host '  -- Step 1/2: Installing NTK binary + Claude Code hook (ntk init -g) --' -ForegroundColor Cyan
Write-Host ''
& $tmp init -g
$initExit = $LASTEXITCODE
Write-Host ''

# Cleanup the throwaway downloader.
Remove-Item -Force -ErrorAction SilentlyContinue $tmp

if ($initExit -ne 0) {
    Write-Host "  ntk init -g failed (exit $initExit). Aborting." -ForegroundColor Red
    throw "ntk init -g failed"
}

Write-Host "  NTK $latest installed to $dest" -ForegroundColor Green
Write-Host ''

if ($postNote -eq 'AMD') {
    Write-Host '  Warning: AMD GPU note: inference uses llama-server + Vulkan (external).' -ForegroundColor Yellow
    Write-Host '    Install llama.cpp (Vulkan build) from:'
    Write-Host '      https://github.com/ggerganov/llama.cpp/releases'
    Write-Host "    Place llama-server.exe on your PATH (or %USERPROFILE%\.ntk\bin\) before"
    Write-Host "    running 'ntk model setup' — choose 'llama.cpp' backend."
    Write-Host ''
}

# ---------------------------------------------------------------------------
# Step 2 — ntk model setup  (configure backend + GPU)
# ---------------------------------------------------------------------------
Write-Host '  -- Step 2/2: Configuring inference backend (ntk model setup) --' -ForegroundColor Cyan
Write-Host ''
& $dest model setup
Write-Host ''
Write-Host '  Installation complete. Run  ntk start  to launch the daemon.' -ForegroundColor Green
Write-Host ''

# Keep the terminal open so the user can read the output.
Write-Host '  Press Enter to close...' -NoNewline
$null = Read-Host
