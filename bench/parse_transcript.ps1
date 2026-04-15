#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Parse a Claude Code session transcript (JSONL) and emit a CSV of
    per-turn token usage plus a final totals row.

.DESCRIPTION
    Claude Code writes each session to
    ~/.claude/projects/<project>/<session-id>.jsonl. Each line is one
    JSON event. The relevant events are `type: assistant` -- they carry
    `message.usage.{input_tokens, cache_creation_input_tokens,
    cache_read_input_tokens, output_tokens}`.

    This script sums those fields across all assistant turns and
    writes:
      turn, model, input_tokens, cache_creation_input_tokens,
      cache_read_input_tokens, output_tokens, total_tokens, timestamp
    Plus a final TOTAL row.

.PARAMETER Transcript
    Path to the .jsonl transcript file.

.PARAMETER Output
    Path to the output CSV. Default: <transcript>.csv beside the input.
#>
param(
    [Parameter(Mandatory)][string]$Transcript,
    [string]$Output = ''
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Transcript)) {
    Write-Error "transcript not found: $Transcript"
    exit 1
}

if ([string]::IsNullOrEmpty($Output)) {
    $Output = [System.IO.Path]::ChangeExtension($Transcript, '.csv')
}

$header = 'turn,model,input_tokens,cache_creation_input_tokens,cache_read_input_tokens,output_tokens,total_tokens,timestamp'
Set-Content -Path $Output -Value $header -Encoding UTF8

$turn = 0
$totals = @{
    input = 0
    cache_create = 0
    cache_read = 0
    output = 0
}

Get-Content $Transcript | ForEach-Object {
    if (-not $_) { return }
    try {
        $event = $_ | ConvertFrom-Json -ErrorAction Stop
    } catch {
        return
    }
    if ($event.type -ne 'assistant') { return }
    if (-not $event.message -or -not $event.message.usage) { return }

    $u = $event.message.usage
    $turn++

    $inT  = if ($u.input_tokens)                 { [int]$u.input_tokens }                else { 0 }
    $ccT  = if ($u.cache_creation_input_tokens)  { [int]$u.cache_creation_input_tokens } else { 0 }
    $crT  = if ($u.cache_read_input_tokens)      { [int]$u.cache_read_input_tokens }     else { 0 }
    $outT = if ($u.output_tokens)                { [int]$u.output_tokens }               else { 0 }
    $total = $inT + $ccT + $crT + $outT

    $totals.input        += $inT
    $totals.cache_create += $ccT
    $totals.cache_read   += $crT
    $totals.output       += $outT

    $model = if ($event.message.model) { $event.message.model } else { 'unknown' }
    $ts = if ($event.timestamp) { $event.timestamp } else { '' }

    $row = @($turn, $model, $inT, $ccT, $crT, $outT, $total, $ts) -join ','
    Add-Content -Path $Output -Value $row -Encoding UTF8
}

$grandTotal = $totals.input + $totals.cache_create + $totals.cache_read + $totals.output
$totalRow = @('TOTAL','', $totals.input, $totals.cache_create, $totals.cache_read, $totals.output, $grandTotal, '') -join ','
Add-Content -Path $Output -Value $totalRow -Encoding UTF8

Write-Host ''
Write-Host ("  parsed {0} assistant turns from {1}" -f $turn, (Split-Path $Transcript -Leaf))
Write-Host ('  input_tokens               : {0,10:N0}' -f $totals.input)
Write-Host ('  cache_creation_input_tokens: {0,10:N0}' -f $totals.cache_create)
Write-Host ('  cache_read_input_tokens    : {0,10:N0}' -f $totals.cache_read)
Write-Host ('  output_tokens              : {0,10:N0}' -f $totals.output)
Write-Host ('  ---------- TOTAL ------------- {0,10:N0}' -f $grandTotal)
Write-Host ''
Write-Host "wrote: $Output"
