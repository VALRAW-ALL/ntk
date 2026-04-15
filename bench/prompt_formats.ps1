#!/usr/bin/env pwsh
<#
.SYNOPSIS
    A/B benchmark of Layer 4 prompt formats (Prefix / XmlWrap / Goal / Json).

.DESCRIPTION
    Runs four independent daemons — one per NTK_L4_FORMAT value — each
    against the same fixture picked for its L3-triggering profile, and
    records the resulting token ratios plus the compressed output.

    Intended to run after `ntk stop`, so the script owns the daemon
    lifecycle. Restarts the daemon between each variant to ensure the
    env var takes effect (env is read at request time but the daemon
    process needs to inherit it on spawn).

    Output: bench/prompt_formats.csv with columns:
      format, fixture, tokens_before, tokens_after, ratio, layer, latency_ms

.PARAMETER Fixtures
    Which fixtures to run (defaults to the 3 that trigger L3).
.PARAMETER TimeoutSec
    Per-request timeout. Default 300 (5 min for CPU L3).
.PARAMETER Context
    Injected user intent used across all formats. Default is a
    typical debugging request.
#>
param(
    [string[]]$Fixtures = @('cargo_build_verbose','generic_long_log','stack_trace_java'),
    [int]$TimeoutSec = 300,
    [string]$Context = 'I am debugging a compilation error in my Rust project. Focus on the error messages and where they come from in the source.'
)

$ErrorActionPreference = 'Stop'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$fixtureDir = Join-Path $here 'fixtures'
$outCsv = Join-Path $here 'prompt_formats.csv'

$formats = @('prefix','xml','goal','json')

function Wait-Healthy {
    param([int]$Seconds = 10)
    for ($i = 0; $i -lt $Seconds; $i++) {
        try {
            $h = Invoke-RestMethod -Uri 'http://127.0.0.1:8765/health' -TimeoutSec 2 -ErrorAction Stop
            if ($h.status -eq 'ok') { return $true }
        } catch {}
        Start-Sleep -Seconds 1
    }
    return $false
}

function Post-Compress {
    param(
        [string]$FixturePath,
        [string]$Context,
        [int]$TimeoutSec
    )
    $content = [System.IO.File]::ReadAllText($FixturePath)
    $payload = @{
        output  = $content
        command = 'bench'
        cwd     = 'bench'
        context = $Context
    } | ConvertTo-Json -Compress -Depth 5

    $bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
    $req   = [System.Net.WebRequest]::Create('http://127.0.0.1:8765/compress')
    $req.Method        = 'POST'
    $req.ContentType   = 'application/json'
    $req.Timeout       = $TimeoutSec * 1000
    $req.ContentLength = $bytes.Length
    $rs = $req.GetRequestStream()
    $rs.Write($bytes, 0, $bytes.Length)
    $rs.Close()
    $t0 = Get-Date
    try {
        $resp = $req.GetResponse()
        $body = (New-Object System.IO.StreamReader($resp.GetResponseStream())).ReadToEnd()
        $resp.Close()
        $elapsed = [int]((Get-Date) - $t0).TotalMilliseconds
        return @{ body = $body; latency = $elapsed; error = '' }
    } catch {
        $elapsed = [int]((Get-Date) - $t0).TotalMilliseconds
        return @{ body = ''; latency = $elapsed; error = $_.Exception.Message }
    }
}

# CSV header
'format,fixture,tokens_before,tokens_after,ratio,layer,latency_ms,error' | Set-Content -Path $outCsv -Encoding UTF8

Write-Host 'Running 4 prompt formats x fixtures' -ForegroundColor Cyan
Write-Host ''

foreach ($fmt in $formats) {
    Write-Host "=== Format: $fmt ===" -ForegroundColor Yellow

    # Restart daemon with the target env var so prompt_format sticks.
    & ntk stop 2>&1 | Out-Null
    Start-Sleep -Seconds 1
    $env:NTK_L4_FORMAT = $fmt
    $env:NTK_LOG_COMPRESSIONS = '1'
    Start-Process -FilePath 'ntk' -ArgumentList 'start' -WindowStyle Hidden
    if (-not (Wait-Healthy -Seconds 10)) {
        Write-Warning "daemon failed to come up for format=$fmt — skipping"
        continue
    }

    foreach ($fixName in $Fixtures) {
        $fxPath = Join-Path $fixtureDir "$fixName.txt"
        if (-not (Test-Path $fxPath)) {
            Write-Warning "fixture missing: $fxPath"
            continue
        }
        $result = Post-Compress -FixturePath $fxPath -Context $Context -TimeoutSec $TimeoutSec
        if ($result.error) {
            Add-Content $outCsv -Value "$fmt,$fixName,,,,,$($result.latency),`"$($result.error -replace ',', ';')`""
            Write-Host ('  {0,-22} ERROR: {1}' -f $fixName, $result.error) -ForegroundColor Red
            continue
        }
        $r = $result.body | ConvertFrom-Json
        $ratio = ([double]$r.ratio).ToString('F3', [System.Globalization.CultureInfo]::InvariantCulture)
        Add-Content $outCsv -Value "$fmt,$fixName,$($r.tokens_before),$($r.tokens_after),$ratio,$($r.layer),$($result.latency),"
        Write-Host ('  {0,-22} {1,6} -> {2,6} ({3}) L{4}  {5} ms' -f `
            $fixName, $r.tokens_before, $r.tokens_after, $ratio, $r.layer, $result.latency)
    }
    Write-Host ''
}

# Stop daemon after experiment
& ntk stop 2>&1 | Out-Null
Remove-Item Env:\NTK_L4_FORMAT -ErrorAction SilentlyContinue

Write-Host "wrote: $outCsv" -ForegroundColor Green
Write-Host ''
Write-Host 'Aggregated ratio by format (higher is better):'
$rows = Import-Csv $outCsv | Where-Object { $_.ratio -ne '' }
$grouped = $rows | Group-Object format | ForEach-Object {
    $avg = ($_.Group | Measure-Object -Property ratio -Average).Average
    [pscustomobject]@{ format = $_.Name; count = $_.Count; avg_ratio = [math]::Round($avg, 3) }
}
$grouped | Sort-Object avg_ratio -Descending | Format-Table -AutoSize
