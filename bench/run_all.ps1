#!/usr/bin/env pwsh
<#
.SYNOPSIS
    One-shot runner for the NTK microbench: generate fixtures (if missing),
    start the daemon with logging, replay, then generate the report.

.DESCRIPTION
    Convenience wrapper. Assumes:
      - `ntk` is on PATH (or pass -NtkBinary path to release/ntk.exe)
      - PowerShell 5.1+ on Windows, or pwsh (Core) anywhere.

.PARAMETER NtkBinary
    Path to the ntk executable. Default: ntk (on PATH).

.PARAMETER TimeoutSec
    Per-request timeout for the replay. Default 300 (5 min) -- L3 on CPU
    can take 60-120s per fixture.

.PARAMETER SkipDaemon
    Skip the daemon-restart step (useful when you already have a daemon
    running with NTK_LOG_COMPRESSIONS=1 set).
#>
param(
    [string]$NtkBinary = 'ntk',
    [int]$TimeoutSec = 300,
    [switch]$SkipDaemon
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

# 1. Ensure fixtures exist
$fixtureCount = (Get-ChildItem (Join-Path $here 'fixtures') -Filter '*.txt' -ErrorAction SilentlyContinue).Count
if ($fixtureCount -lt 8) {
    Write-Host '==> Generating fixtures...'
    & (Join-Path $here 'generate_fixtures.ps1')
}

# 2. Restart daemon with logging enabled (unless asked to skip)
if (-not $SkipDaemon) {
    Write-Host '==> Stopping any running daemon...'
    & $NtkBinary stop 2>&1 | Out-Null

    Write-Host '==> Starting daemon with NTK_LOG_COMPRESSIONS=1 in background...'
    $env:NTK_LOG_COMPRESSIONS = '1'
    Start-Process -FilePath $NtkBinary -ArgumentList 'start' -WindowStyle Hidden

    # Wait up to 10 s for health check
    $up = $false
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Seconds 1
        try {
            $h = Invoke-RestMethod -Uri 'http://127.0.0.1:8765/health' -TimeoutSec 2 -ErrorAction Stop
            if ($h.status -eq 'ok') { $up = $true; break }
        } catch {}
    }
    if (-not $up) {
        Write-Error 'daemon did not come up in 10s -- check `ntk start` manually'
        exit 1
    }
}

# 3. Replay fixtures
Write-Host '==> Running microbench (this may take several minutes)...'
& (Join-Path $here 'replay.ps1') -TimeoutSec $TimeoutSec

# 4. Report
Write-Host ''
Write-Host '==> Generating report...'
& (Join-Path $here 'report.ps1')

Write-Host ''
Write-Host '==> Done.'
Write-Host '    microbench.csv : ' (Join-Path $here 'microbench.csv')
Write-Host '    report.md      : ' (Join-Path $here 'report.md')
Write-Host '    logs           : ' (Join-Path $env:USERPROFILE '.ntk\logs')
