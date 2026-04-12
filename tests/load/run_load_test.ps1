# NTK Load Test -- exercises L1, L2, and L3 by POSTing directly to the daemon.
# Usage: powershell -File tests/load/run_load_test.ps1

$NtkUrl    = if ($env:NTK_DAEMON_URL) { $env:NTK_DAEMON_URL } else { "http://127.0.0.1:8765" }
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Sep       = "=" * 60

function Write-Header($text) {
    Write-Host ""
    Write-Host $Sep -ForegroundColor DarkGreen
    Write-Host "  $text" -ForegroundColor Green
    Write-Host $Sep -ForegroundColor DarkGreen
}

function Test-Layer($name, $command) {
    Write-Host ""
    Write-Host "  [$name]" -ForegroundColor Cyan -NoNewline
    Write-Host " generating payload..." -ForegroundColor DarkGray

    $output    = (& powershell -NoProfile -File "$ScriptDir\$command") -join "`n"
    $charCount = $output.Length

    Write-Host "    chars     : $charCount" -ForegroundColor DarkGray

    if ($charCount -lt 500) {
        Write-Host "    SKIP: payload < 500 chars" -ForegroundColor Yellow
        return
    }

    $body = @{
        output  = $output
        command = "load-test-$($name.ToLower())"
        cwd     = "C:\Projetos\ntk"
    } | ConvertTo-Json -Compress -Depth 5

    try {
        $resp = Invoke-RestMethod `
            -Uri "$NtkUrl/compress" `
            -Method Post `
            -ContentType "application/json" `
            -Body $body `
            -TimeoutSec 30
    } catch {
        Write-Host "    ERROR: $($_.Exception.Message)" -ForegroundColor Red
        return
    }

    $savedTok = $resp.tokens_before - $resp.tokens_after
    $pct      = [math]::Round($resp.ratio * 100, 1)
    $layer    = $resp.layer

    $layerColor = switch ($layer) {
        1       { "Green"   }
        2       { "Cyan"    }
        3       { "Magenta" }
        default { "White"   }
    }

    Write-Host "    tokens_in : $($resp.tokens_before)" -ForegroundColor White
    Write-Host "    tokens_out: $($resp.tokens_after)" -ForegroundColor Green
    Write-Host "    saved     : $savedTok tokens ($($pct)%)" -ForegroundColor Green
    Write-Host "    layer     : L$layer" -ForegroundColor $layerColor
    Write-Host ""
    Write-Host "  --- Compressed (first 500 chars) ---" -ForegroundColor DarkGray
    $preview = if ($resp.compressed.Length -gt 500) {
        $resp.compressed.Substring(0, 500) + "..."
    } else {
        $resp.compressed
    }
    Write-Host $preview -ForegroundColor Gray
}

# -- Health check -------------------------------------------------------------
Write-Header "NTK Load Test"
Write-Host ""
Write-Host "  Daemon : $NtkUrl" -ForegroundColor DarkGray

try {
    $health = Invoke-RestMethod -Uri "$NtkUrl/health" -TimeoutSec 5
    Write-Host "  Status : UP  (uptime $($health.uptime_secs)s, backend: $($health.backend))" -ForegroundColor Green
} catch {
    Write-Host "  Status : DOWN -- start the daemon first with: ntk start" -ForegroundColor Red
    exit 1
}

# -- Snapshot metrics before --------------------------------------------------
$before = Invoke-RestMethod -Uri "$NtkUrl/metrics" -TimeoutSec 5

# -- Run each layer test ------------------------------------------------------
Write-Header "Layer 1 -- Fast Filter (ANSI strip + line dedup)"
Test-Layer "L1" "gen_l1_payload.ps1"

Write-Header "Layer 2 -- Tokenizer-Aware (BPE path shortening)"
Test-Layer "L2" "gen_l2_payload.ps1"

Write-Header "Layer 3 -- Semantic Inference (Ollama/Candle/llama.cpp)"
Test-Layer "L3" "gen_l3_payload.ps1"

# -- Session delta ------------------------------------------------------------
Write-Header "Session Delta (this run)"

try {
    $after           = Invoke-RestMethod -Uri "$NtkUrl/metrics" -TimeoutSec 5
    $newCompressions = $after.total_compressions - $before.total_compressions
    $newSaved        = $after.total_tokens_saved  - $before.total_tokens_saved
    $avgPct          = [math]::Round($after.average_ratio * 100, 1)

    Write-Host ""
    Write-Host "  New compressions : $newCompressions" -ForegroundColor White
    Write-Host "  New tokens saved : $newSaved" -ForegroundColor Green
    Write-Host "  Avg ratio (all)  : $($avgPct)%" -ForegroundColor Green
    Write-Host ""
} catch {
    Write-Host "  Could not fetch metrics: $($_.Exception.Message)" -ForegroundColor Yellow
}

Write-Host $Sep -ForegroundColor DarkGreen
Write-Host ""
