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
    [string]$Context = 'I am debugging a compilation error in my Rust project. Focus on the error messages and where they come from in the source.',
    # When true (default), each (format, fixture) pair is run twice:
    # once with the user intent and once with an empty context. The CSV
    # gains a `context_enabled` column and the summary reports the delta.
    # This is the experiment #5 asks for: proving L4 earns its latency.
    [switch]$CompareContext = $true
)

$ErrorActionPreference = 'Stop'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$fixtureDir = Join-Path $here 'fixtures'
$outCsv = Join-Path $here 'prompt_formats.csv'

$formats = @('prefix','xml','goal','json')

# Daemon auth token (see issue #2). Written to ~/.ntk/.token by the
# daemon on first start; the bench script must send it as X-NTK-Token
# or every request returns 401. When NTK_DISABLE_AUTH=1 is set on the
# daemon, the token is irrelevant and an empty string is fine.
$NtkTokenFile = Join-Path $env:USERPROFILE '.ntk\.token'
$NtkToken = ''
if (Test-Path $NtkTokenFile) {
    try { $NtkToken = (Get-Content -Raw -LiteralPath $NtkTokenFile).Trim() } catch {}
}

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
    if ($script:NtkToken) {
        $req.Headers.Add('X-NTK-Token', $script:NtkToken)
    }
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

# CSV header — context_enabled and context_kept_err_signal give the two
# dimensions the experiment needs: impact on ratio AND on information
# preservation (naive regex for lines that look like errors).
'format,fixture,context_enabled,tokens_before,tokens_after,ratio,layer,latency_ms,error' | Set-Content -Path $outCsv -Encoding UTF8

# Each fixture gets 1 run when CompareContext is off, 2 runs when on.
$contextVariants = if ($CompareContext) { @($true, $false) } else { @($true) }

Write-Host ('Running {0} prompt formats x {1} fixtures x {2} context variants' -f `
    $formats.Count, $Fixtures.Count, $contextVariants.Count) -ForegroundColor Cyan
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
        foreach ($ctxEnabled in $contextVariants) {
            $ctx = if ($ctxEnabled) { $Context } else { '' }
            $result = Post-Compress -FixturePath $fxPath -Context $ctx -TimeoutSec $TimeoutSec
            if ($result.error) {
                Add-Content $outCsv -Value "$fmt,$fixName,$ctxEnabled,,,,,$($result.latency),`"$($result.error -replace ',', ';')`""
                Write-Host ('  {0,-22} ctx={1,-5} ERROR: {2}' -f $fixName, $ctxEnabled, $result.error) -ForegroundColor Red
                continue
            }
            $r = $result.body | ConvertFrom-Json
            $ratio = ([double]$r.ratio).ToString('F3', [System.Globalization.CultureInfo]::InvariantCulture)
            Add-Content $outCsv -Value "$fmt,$fixName,$ctxEnabled,$($r.tokens_before),$($r.tokens_after),$ratio,$($r.layer),$($result.latency),"
            Write-Host ('  {0,-22} ctx={1,-5} {2,6} -> {3,6} ({4}) L{5}  {6} ms' -f `
                $fixName, $ctxEnabled, $r.tokens_before, $r.tokens_after, $ratio, $r.layer, $result.latency)
        }
    }
    Write-Host ''
}

# Stop daemon after experiment
& ntk stop 2>&1 | Out-Null
Remove-Item Env:\NTK_L4_FORMAT -ErrorAction SilentlyContinue

Write-Host "wrote: $outCsv" -ForegroundColor Green
Write-Host ''

$rows = Import-Csv $outCsv | Where-Object { $_.ratio -ne '' }

Write-Host 'Aggregated ratio by format (higher is better):'
$byFormat = $rows | Group-Object format | ForEach-Object {
    $avg = ($_.Group | Measure-Object -Property ratio -Average).Average
    [pscustomobject]@{ format = $_.Name; count = $_.Count; avg_ratio = [math]::Round($avg, 3) }
}
$byFormat | Sort-Object avg_ratio -Descending | Format-Table -AutoSize

if ($CompareContext) {
    Write-Host 'Context impact — average ratio with vs without L4 context:' -ForegroundColor Cyan
    $byCtx = $rows | Group-Object format,context_enabled | ForEach-Object {
        $parts = $_.Name -split ', '
        $avg = ($_.Group | Measure-Object -Property ratio -Average).Average
        [pscustomobject]@{
            format          = $parts[0]
            context_enabled = $parts[1]
            count           = $_.Count
            avg_ratio       = [math]::Round($avg, 3)
        }
    }
    $byCtx | Sort-Object format,context_enabled | Format-Table -AutoSize

    # Delta per format: ratio(with context) - ratio(without).
    # Positive = context helps; negative = context hurts (candidate to disable).
    Write-Host 'Delta (with context — without context), per format:' -ForegroundColor Cyan
    $pairs = $rows | Group-Object format,fixture
    $deltas = foreach ($pair in $pairs) {
        $with    = $pair.Group | Where-Object { $_.context_enabled -eq 'True' }
        $without = $pair.Group | Where-Object { $_.context_enabled -eq 'False' }
        if ($with -and $without) {
            $parts = $pair.Name -split ', '
            [pscustomobject]@{
                format  = $parts[0]
                fixture = $parts[1]
                delta   = [math]::Round(([double]$with[0].ratio - [double]$without[0].ratio), 3)
            }
        }
    }
    $deltas | Sort-Object format,fixture | Format-Table -AutoSize

    $summary = $deltas | Group-Object format | ForEach-Object {
        $avg = ($_.Group | Measure-Object -Property delta -Average).Average
        [pscustomobject]@{ format = $_.Name; avg_delta = [math]::Round($avg, 3) }
    }
    Write-Host 'Average delta per format (positive = context helps):' -ForegroundColor Yellow
    $summary | Sort-Object avg_delta -Descending | Format-Table -AutoSize
}
