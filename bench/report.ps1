#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Aggregate microbench + A/B session CSVs into a markdown report.

.DESCRIPTION
    Inputs:
      - bench/microbench.csv (from replay.ps1)
      - variant A transcript CSV (hook OFF) from parse_transcript.ps1
      - variant B transcript CSV (hook ON)  from parse_transcript.ps1

    Output: bench/report.md with tables:
      1. Per-fixture ratios + which layer won.
      2. A-vs-B session totals and delta (tokens + estimated USD cost).
      3. Per-category L1/L2/L3 contribution breakdown.
      4. Flags: fixtures with low/high ratios, any errors.

.PARAMETER Micro
    Path to microbench.csv. Default: bench/microbench.csv.

.PARAMETER A
    Path to variant A (hook OFF) transcript CSV.

.PARAMETER B
    Path to variant B (hook ON) transcript CSV.

.PARAMETER Out
    Output markdown path. Default: bench/report.md.

.PARAMETER SonnetInputRate
    USD per 1K input tokens. Default 0.003 (Sonnet 4.6).

.PARAMETER SonnetOutputRate
    USD per 1K output tokens. Default 0.015 (Sonnet 4.6).

.PARAMETER SonnetCacheReadRate
    USD per 1K cache read tokens. Default 0.0003 (10% of input).

.PARAMETER SonnetCacheCreateRate
    USD per 1K cache creation tokens. Default 0.00375 (1.25x of input).
#>
param(
    [string]$Micro = '',
    [string]$A = '',
    [string]$B = '',
    [string]$Out = '',
    [double]$SonnetInputRate = 0.003,
    [double]$SonnetOutputRate = 0.015,
    [double]$SonnetCacheReadRate = 0.0003,
    [double]$SonnetCacheCreateRate = 0.00375
)

$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrEmpty($Micro)) { $Micro = Join-Path $here 'microbench.csv' }
if ([string]::IsNullOrEmpty($Out))   { $Out   = Join-Path $here 'report.md' }

function Read-Csv-Safe($path) {
    if ([string]::IsNullOrEmpty($path) -or -not (Test-Path $path)) { return $null }
    return Import-Csv -Path $path
}

function Fmt-Pct($ratio) {
    if ([string]::IsNullOrEmpty("$ratio") -or $ratio -eq '0.000') { return '0%' }
    $r = [double]::Parse($ratio, [System.Globalization.CultureInfo]::InvariantCulture)
    return '{0:P0}' -f $r
}

function Fmt-Int($n) {
    if ([string]::IsNullOrEmpty("$n")) { return '-' }
    return ([long]$n).ToString('N0', [System.Globalization.CultureInfo]::InvariantCulture)
}

function Fmt-Usd($v) {
    return ([double]$v).ToString('F4', [System.Globalization.CultureInfo]::InvariantCulture)
}

# ---- Section 1: Microbench ------------------------------------------
$microRows = Read-Csv-Safe $Micro
if (-not $microRows) {
    Write-Error "microbench CSV not found or empty: $Micro"
    exit 1
}

$microTable = @()
$lowRatio = @()
$highRatio = @()
$errors = @()
$layerTallies = @{ L0 = 0; L1 = 0; L2 = 0; L3 = 0 }
foreach ($row in $microRows) {
    if ($row.http_status -ne '200' -or [string]::IsNullOrEmpty($row.ratio)) {
        $errors += $row
        continue
    }
    $ratio = [double]::Parse($row.ratio, [System.Globalization.CultureInfo]::InvariantCulture)
    $layerTallies."L$($row.layer_used)" += 1
    $microTable += [pscustomobject]@{
        fixture    = $row.fixture
        before     = $row.tokens_before
        l1         = $row.tokens_after_l1
        l2         = $row.tokens_after_l2
        l3         = if ($row.tokens_after_l3) { $row.tokens_after_l3 } else { '-' }
        final      = $row.tokens_after
        layer      = "L$($row.layer_used)"
        ratio_pct  = '{0:P0}' -f $ratio
        latency_ms = $row.latency_ms_total
    }
    if ($ratio -lt 0.20 -and $row.fixture -ne 'already_short') { $lowRatio += $row.fixture }
    if ($ratio -gt 0.85)                                        { $highRatio += $row.fixture }
}

# ---- Sections 2+3: A vs B sessions ---------------------------------
$aRows = Read-Csv-Safe $A
$bRows = Read-Csv-Safe $B

function Get-Totals($rows) {
    if (-not $rows) { return $null }
    $total = $rows | Where-Object { $_.turn -eq 'TOTAL' } | Select-Object -First 1
    if (-not $total) { return $null }
    return [pscustomobject]@{
        turns  = ($rows | Where-Object { $_.turn -ne 'TOTAL' }).Count
        input  = [long]$total.input_tokens
        cc     = [long]$total.cache_creation_input_tokens
        cr     = [long]$total.cache_read_input_tokens
        output = [long]$total.output_tokens
        total  = [long]$total.total_tokens
    }
}

$aTotals = Get-Totals $aRows
$bTotals = Get-Totals $bRows

function Get-Cost($t) {
    if (-not $t) { return 0.0 }
    $c = ($t.input  / 1000.0) * $SonnetInputRate
    $c += ($t.cc     / 1000.0) * $SonnetCacheCreateRate
    $c += ($t.cr     / 1000.0) * $SonnetCacheReadRate
    $c += ($t.output / 1000.0) * $SonnetOutputRate
    return $c
}

$aCost = Get-Cost $aTotals
$bCost = Get-Cost $bTotals

# ---- Build markdown ------------------------------------------------
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("# NTK Benchmark Report")
$lines.Add("")
$lines.Add("Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
$lines.Add("")
$lines.Add("## 1. Microbench -- per-fixture compression")
$lines.Add("")
$lines.Add("Each fixture posted once against the running daemon. Tokens counted with the")
$lines.Add("`cl100k_base` tokenizer (same one Claude uses).")
$lines.Add("")
$lines.Add('| Fixture | Before | L1 | L2 | L3 | Final | Layer | Savings | Latency |')
$lines.Add('|---|---:|---:|---:|---:|---:|:---:|---:|---:|')
foreach ($r in $microTable) {
    $lines.Add(
        ('| `{0}` | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} ms |' -f `
            $r.fixture, (Fmt-Int $r.before), (Fmt-Int $r.l1),
            (Fmt-Int $r.l2), $r.l3, (Fmt-Int $r.final),
            $r.layer, $r.ratio_pct, $r.latency_ms)
    )
}
$lines.Add("")
$lines.Add(('**Layer distribution:** L0={0}, L1={1}, L2={2}, L3={3}' -f `
    $layerTallies.L0, $layerTallies.L1, $layerTallies.L2, $layerTallies.L3))
$lines.Add("")

if ($errors.Count -gt 0) {
    $lines.Add("### Errors")
    $lines.Add("")
    foreach ($e in $errors) {
        $lines.Add(('- `{0}` -- HTTP {1}: {2}' -f $e.fixture, $e.http_status, $e.error))
    }
    $lines.Add("")
}

if ($highRatio.Count -gt 0) {
    $lines.Add(('### Big wins (ratio > 85%): ' + ($highRatio -join ', ')))
    $lines.Add("")
}
if ($lowRatio.Count -gt 0) {
    $lines.Add(('### Marginal (ratio < 20%, excluding `already_short`): ' + ($lowRatio -join ', ')))
    $lines.Add(('> These fixtures barely benefit from NTK -- consider disabling compression for similar commands.'))
    $lines.Add("")
}

# ---- Section 2: Macro A vs B --------------------------------------
$lines.Add("## 2. End-to-end: Claude Code session A (hook OFF) vs B (hook ON)")
$lines.Add("")
if (-not $aTotals -or -not $bTotals) {
    $lines.Add("*Provide -A and -B paths to include the A-vs-B comparison.*")
    $lines.Add("")
} else {
    $lines.Add('| Metric | A (hook OFF) | B (hook ON) | Delta | Delta % |')
    $lines.Add('|---|---:|---:|---:|---:|')
    function DeltaRow($name, $av, $bv) {
        $delta = $bv - $av
        $pct = if ($av -eq 0) { '-' } else { '{0:P1}' -f ($delta / $av) }
        ('| {0} | {1} | {2} | {3} | {4} |' -f $name, (Fmt-Int $av), (Fmt-Int $bv), (Fmt-Int $delta), $pct)
    }
    $lines.Add((DeltaRow 'Turns'                $aTotals.turns  $bTotals.turns))
    $lines.Add((DeltaRow 'input_tokens'         $aTotals.input  $bTotals.input))
    $lines.Add((DeltaRow 'cache_creation'       $aTotals.cc     $bTotals.cc))
    $lines.Add((DeltaRow 'cache_read'           $aTotals.cr     $bTotals.cr))
    $lines.Add((DeltaRow 'output_tokens'        $aTotals.output $bTotals.output))
    $lines.Add((DeltaRow '**TOTAL TOKENS**'     $aTotals.total  $bTotals.total))
    $lines.Add("")
    $deltaCost = $bCost - $aCost
    $deltaCostPct = if ($aCost -eq 0) { '-' } else { '{0:P1}' -f ($deltaCost / $aCost) }
    $lines.Add(('**Estimated cost (Sonnet 4.6 rates):** A = **$' + (Fmt-Usd $aCost) + '**  |  B = **$' + (Fmt-Usd $bCost) + '**  |  Delta = **$' + (Fmt-Usd $deltaCost) + '** (' + $deltaCostPct + ')'))
    $lines.Add("")

    # Plain-text verdict
    $verdict = ''
    if ($bTotals.total -eq 0) {
        $verdict = '❓ variant B is empty'
    } elseif ($bTotals.total -lt $aTotals.total * 0.9) {
        $verdict = '✅ **Net savings** -- NTK hook reduced total tokens by more than 10%.'
    } elseif ($bTotals.total -lt $aTotals.total) {
        $verdict = '➖ **Marginal savings** -- NTK reduced tokens by <10%.'
    } elseif ($bTotals.total -gt $aTotals.total * 1.05) {
        $verdict = '⚠️ **Overhead without benefit** -- NTK increased total tokens. Check error log; L3 inference tokens may be leaking into the transcript.'
    } else {
        $verdict = '➖ **Neutral** -- within ±5%, statistically a wash.'
    }
    $lines.Add('**Verdict:** ' + $verdict)
    $lines.Add("")
}

# ---- Section 3: Rates used ----------------------------------------
$lines.Add("## 3. Pricing assumptions")
$lines.Add("")
$lines.Add("USD per 1K tokens (edit via script flags):")
function Fmt-Rate($v) { return ([double]$v).ToString('F5', [System.Globalization.CultureInfo]::InvariantCulture) }
$lines.Add(('- input: **$' + (Fmt-Rate $SonnetInputRate) + '**'))
$lines.Add(('- output: **$' + (Fmt-Rate $SonnetOutputRate) + '**'))
$lines.Add(('- cache read: **$' + (Fmt-Rate $SonnetCacheReadRate) + '**'))
$lines.Add(('- cache creation: **$' + (Fmt-Rate $SonnetCacheCreateRate) + '**'))
$lines.Add("")
$lines.Add("> Anthropic updates prices at https://www.anthropic.com/pricing.")
$lines.Add('> Pass rates explicitly when running: report.ps1 -SonnetInputRate 0.003 ...')
$lines.Add("")

# ---- Write ---------------------------------------------------------
[System.IO.File]::WriteAllText($Out, ($lines -join [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))

Write-Host ''
Write-Host "wrote: $Out"
Write-Host ("  microbench rows: {0}" -f $microRows.Count)
if ($aTotals) { Write-Host ("  A (hook OFF): {0} turns, {1:N0} tokens" -f $aTotals.turns, $aTotals.total) }
if ($bTotals) { Write-Host ("  B (hook ON) : {0} turns, {1:N0} tokens" -f $bTotals.turns, $bTotals.total) }
