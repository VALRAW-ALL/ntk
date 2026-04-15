#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Replay every NTK fixture against the live daemon and produce microbench.csv.

.DESCRIPTION
    For each bench/fixtures/*.txt file, POSTs the content to
    http://127.0.0.1:8765/compress and records:
      fixture, bytes_in, tokens_before, tokens_after_l1, tokens_after_l2,
      tokens_after_l3, tokens_after, layer_used, ratio, latency_ms_total,
      latency_ms_l1, latency_ms_l2, latency_ms_l3

    Prerequisites:
      - NTK daemon running on 127.0.0.1:8765 (`ntk start`).
      - Optional but recommended: set NTK_LOG_COMPRESSIONS=1 before
        starting the daemon so raw input + per-layer outputs are
        persisted to ~/.ntk/logs/.

.PARAMETER DaemonUrl
    Override the daemon URL. Defaults to http://127.0.0.1:8765.

.PARAMETER OutputCsv
    Path to the output CSV. Defaults to bench/microbench.csv.

.PARAMETER TimeoutSec
    Per-request timeout in seconds. Defaults to 300 (5 min) -- L3
    inference on CPU can take 60-120 s.
#>
param(
    [string]$DaemonUrl = 'http://127.0.0.1:8765',
    [string]$OutputCsv = '',
    [int]$TimeoutSec = 300
)

$ErrorActionPreference = 'Stop'

$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$fixtures = Join-Path $here 'fixtures'
if ([string]::IsNullOrEmpty($OutputCsv)) {
    $OutputCsv = Join-Path $here 'microbench.csv'
}

# Sanity: daemon reachable?
try {
    $health = Invoke-RestMethod -Uri "$DaemonUrl/health" -TimeoutSec 3
    Write-Host ("  daemon: {0} v{1}  model={2}  uptime={3}s" -f `
        $health.status, $health.version, $health.model, $health.uptime_secs)
} catch {
    Write-Error "daemon unreachable at $DaemonUrl -- start it with ``ntk start``"
    exit 1
}

# CSV header
$header = @(
    'fixture','bytes_in','tokens_before','tokens_after_l1','tokens_after_l2',
    'tokens_after_l3','tokens_after','layer_used','ratio','latency_ms_total',
    'latency_ms_l1','latency_ms_l2','latency_ms_l3','http_status','error'
) -join ','
Set-Content -Path $OutputCsv -Value $header -Encoding UTF8

$rawFiles = [System.IO.Directory]::GetFiles($fixtures, '*.txt')
[Array]::Sort($rawFiles)
$filePaths = @($rawFiles)
if ($filePaths.Count -lt 1) {
    Write-Error "no fixtures found in $fixtures -- run generate_fixtures.ps1 first"
    exit 1
}

Write-Host ''
Write-Host ('{0,-40} {1,10} {2,10} {3,6} {4,8} {5,8}' -f `
    'fixture','before','after','L','ratio','ms')
Write-Host ('-' * 88)

foreach ($fxPath in $filePaths) {
    $name = [System.IO.Path]::GetFileNameWithoutExtension($fxPath)
    $content = [System.IO.File]::ReadAllText($fxPath)
    $bytesIn = (New-Object System.IO.FileInfo($fxPath)).Length

    # Read metadata for the command field
    $metaPath = Join-Path $fixtures "$name.meta.json"
    $cmd = 'unknown'
    if (Test-Path $metaPath) {
        $meta = Get-Content $metaPath -Raw | ConvertFrom-Json
        if ($meta.command) { $cmd = $meta.command }
    }

    $payload = @{
        output  = $content
        command = $cmd
        cwd     = 'bench'
    } | ConvertTo-Json -Compress -Depth 5

    $tmpPayload = [System.IO.Path]::Combine($env:TEMP, "ntk_bench_$($name).json")
    [System.IO.File]::WriteAllText($tmpPayload, $payload, [System.Text.UTF8Encoding]::new($false))

    $t0 = Get-Date
    $httpStatus = 0
    $errMsg = ''
    $resp = $null
    try {
        # System.Net.WebRequest allows long timeouts without async wrappers.
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
        $req = [System.Net.WebRequest]::Create("$DaemonUrl/compress")
        $req.Method        = 'POST'
        $req.ContentType   = 'application/json'
        $req.Timeout       = $TimeoutSec * 1000
        $req.ContentLength = $bytes.Length
        $rs = $req.GetRequestStream()
        $rs.Write($bytes, 0, $bytes.Length)
        $rs.Close()
        $response = $req.GetResponse()
        $httpStatus = [int]($response.StatusCode)
        $reader = New-Object System.IO.StreamReader($response.GetResponseStream())
        $body = $reader.ReadToEnd()
        $reader.Close()
        $response.Close()
        $resp = $body | ConvertFrom-Json
    } catch [System.Net.WebException] {
        $errMsg = $_.Exception.Message -replace ',', ';'
        if ($_.Exception.Response) {
            $httpStatus = [int]($_.Exception.Response.StatusCode)
        }
    } catch {
        $errMsg = $_.Exception.Message -replace ',', ';'
    }
    $elapsed = [int]((Get-Date) - $t0).TotalMilliseconds

    Remove-Item $tmpPayload -ErrorAction SilentlyContinue

    # Extract fields with empty-string fallback for CSV safety
    $tBefore = if ($resp) { $resp.tokens_before } else { '' }
    $tL1     = if ($resp -and $resp.tokens_after_l1) { $resp.tokens_after_l1 } else { '' }
    $tL2     = if ($resp -and $resp.tokens_after_l2) { $resp.tokens_after_l2 } else { '' }
    $tL3     = if ($resp -and $resp.tokens_after_l3) { $resp.tokens_after_l3 } else { '' }
    $tAfter  = if ($resp) { $resp.tokens_after } else { '' }
    $layer   = if ($resp) { $resp.layer } else { '' }
    $ratio   = if ($resp) { ([double]$resp.ratio).ToString('F3', [System.Globalization.CultureInfo]::InvariantCulture) } else { '' }
    $msL1    = if ($resp -and $resp.latency_ms.l1) { $resp.latency_ms.l1 } else { '' }
    $msL2    = if ($resp -and $resp.latency_ms.l2) { $resp.latency_ms.l2 } else { '' }
    $msL3    = if ($resp -and $resp.latency_ms.l3) { $resp.latency_ms.l3 } else { '' }

    $row = @(
        $name, $bytesIn, $tBefore, $tL1, $tL2, $tL3, $tAfter,
        $layer, $ratio, $elapsed, $msL1, $msL2, $msL3,
        $httpStatus, ('"' + $errMsg + '"')
    ) -join ','
    Add-Content -Path $OutputCsv -Value $row -Encoding UTF8

    $displayAfter = if ($tAfter -eq '') { 'ERR' } else { "$tAfter" }
    $displayRatio = if ($ratio -eq '')  { '-'   } else { $ratio }
    Write-Host ('{0,-40} {1,10} {2,10} {3,6} {4,8} {5,8}' -f `
        $name, "$tBefore", $displayAfter, "$layer", $displayRatio, "$elapsed")
}

Write-Host ''
Write-Host "wrote: $OutputCsv"
