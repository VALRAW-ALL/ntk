#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Orchestrates the macro A/B experiment: one Claude Code session with
    the NTK hook off, one with it on, then produces a delta report.

.DESCRIPTION
    Claude Code itself is interactive — this script cannot invoke it
    headlessly. It prepares the environment, pauses for you to paste
    the prompt twice (once per variant), then parses the transcripts
    and generates the report.

    Steps:
      1. Variant A (hook OFF):
           - ntk init --uninstall (removes the hook)
           - You open Claude Code, paste bench/prompts/baseline.md, run.
           - Script waits for you to press Enter when done.
           - It copies the latest transcript to bench/transcripts/A.jsonl.
      2. Variant B (hook ON):
           - ntk init -g (installs the hook)
           - ntk start (with NTK_LOG_COMPRESSIONS=1)
           - You open Claude Code again, paste the same prompt, run.
           - Script waits for Enter.
           - It copies the transcript to bench/transcripts/B.jsonl.
      3. Report:
           - parse_transcript.ps1 A/B.jsonl -> A/B.csv
           - report.ps1 -A A.csv -B B.csv -> report.md

.NOTES
    Windows-only. Unix users follow the steps in bench/README.md.
#>
param(
    [string]$ProjectPath = 'C:\Users\Alessandro\Desktop\ntk'
)

$ErrorActionPreference = 'Stop'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$transcripts = Join-Path $here 'transcripts'
New-Item -ItemType Directory -Force -Path $transcripts | Out-Null

$prompt = Join-Path $here 'prompts/baseline.md'
$claudeProjects = Join-Path $env:USERPROFILE '.claude\projects'
# Claude Code project directory naming: each char of the path that isn't a
# letter/digit is replaced by '-'.
$projectKey = ($ProjectPath -replace '[^A-Za-z0-9]', '-')
$sessionDir = Join-Path $claudeProjects $projectKey
if (-not (Test-Path $sessionDir)) {
    Write-Error "Claude Code session dir not found: $sessionDir"
    exit 1
}

function Wait-For-Session {
    param([string]$Label)
    Write-Host ''
    Write-Host "=== $Label ===" -ForegroundColor Cyan
    Write-Host 'Open Claude Code in this directory (ntk repo root).'
    Write-Host 'Paste the contents of:' -NoNewline; Write-Host " $prompt" -ForegroundColor Yellow
    Write-Host 'Run the prompt to completion (step 8 of the baseline is the final summary).'
    Write-Host ''
    Write-Host 'Press Enter when the session has finished...'
    $null = Read-Host
}

function Latest-Transcript {
    Get-ChildItem $sessionDir -Filter '*.jsonl' |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}

# --- Variant A: hook OFF ---
Write-Host 'Step 1: Uninstalling NTK hook for Variant A (baseline)...'
& ntk init --uninstall 2>&1 | Out-Null

Wait-For-Session -Label 'Variant A (NTK hook OFF) — paste baseline prompt'

$latestA = Latest-Transcript
if (-not $latestA) {
    Write-Error 'No Claude Code session transcript found for variant A.'
    exit 1
}
$destA = Join-Path $transcripts 'A.jsonl'
Copy-Item $latestA.FullName $destA -Force
Write-Host ('  Variant A transcript: {0} ({1} bytes)' -f $destA, (Get-Item $destA).Length) -ForegroundColor Green

# --- Variant B: hook ON ---
Write-Host ''
Write-Host 'Step 2: Installing NTK hook for Variant B and starting the daemon...'
& ntk init -g 2>&1 | Out-Null
& ntk stop 2>&1 | Out-Null
$env:NTK_LOG_COMPRESSIONS = '1'
Start-Process -FilePath 'ntk' -ArgumentList 'start' -WindowStyle Hidden
Start-Sleep -Seconds 2
try {
    $h = Invoke-RestMethod -Uri 'http://127.0.0.1:8765/health' -TimeoutSec 3
    Write-Host "  daemon up: $($h.version) $($h.model)" -ForegroundColor Green
} catch {
    Write-Warning 'daemon did not come up quickly — you may need to run `ntk start` manually'
}

Wait-For-Session -Label 'Variant B (NTK hook ON) — paste baseline prompt'

$latestB = Latest-Transcript
if (-not $latestB -or ($latestB.FullName -eq $latestA.FullName)) {
    Write-Error 'No new transcript detected for variant B. Did Claude Code create a new session?'
    exit 1
}
$destB = Join-Path $transcripts 'B.jsonl'
Copy-Item $latestB.FullName $destB -Force
Write-Host ('  Variant B transcript: {0} ({1} bytes)' -f $destB, (Get-Item $destB).Length) -ForegroundColor Green

# --- Parse + report ---
Write-Host ''
Write-Host 'Step 3: Parsing transcripts...' -ForegroundColor Cyan
& (Join-Path $here 'parse_transcript.ps1') -Transcript $destA | Out-Null
& (Join-Path $here 'parse_transcript.ps1') -Transcript $destB | Out-Null

Write-Host ''
Write-Host 'Step 4: Generating report...' -ForegroundColor Cyan
$csvA = [System.IO.Path]::ChangeExtension($destA, '.csv')
$csvB = [System.IO.Path]::ChangeExtension($destB, '.csv')
& (Join-Path $here 'report.ps1') -A $csvA -B $csvB | Out-Null

$report = Join-Path $here 'report.md'
Write-Host ''
Write-Host "Done — see $report" -ForegroundColor Green
