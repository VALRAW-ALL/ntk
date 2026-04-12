# Generates a payload that exercises Layer 3 (semantic inference).
# Pattern: verbose non-repetitive JSON logs -- L1/L2 leave > 300 tokens.

$services = @("auth-svc", "api-gateway", "compression-daemon", "metrics-collector", "telemetry-sink")
$levels   = @("INFO", "DEBUG", "DEBUG", "DEBUG", "WARN")
$methods  = @("GET", "POST", "PUT", "DELETE", "PATCH")
$apiPaths = @("/api/v1/compress", "/api/v1/metrics", "/health", "/api/v1/sessions", "/api/v1/config")
$users    = @("usr_a1b2c3", "usr_d4e5f6", "usr_g7h8i9", "usr_j0k1l2", "usr_m3n4o5")

$lines = [System.Collections.Generic.List[string]]::new()

# 80 structured JSON log lines with unique IDs and values
for ($i = 0; $i -lt 80; $i++) {
    $svc    = $services[(Get-Random -Min 0 -Max $services.Count)]
    $lvl    = $levels[(Get-Random -Min 0 -Max $levels.Count)]
    $method = $methods[(Get-Random -Min 0 -Max $methods.Count)]
    $path   = $apiPaths[(Get-Random -Min 0 -Max $apiPaths.Count)]
    $user   = $users[(Get-Random -Min 0 -Max $users.Count)]
    $dur    = Get-Random -Min 2 -Max 1200
    $tokIn  = Get-Random -Min 120 -Max 8000
    $ratio  = [math]::Round((Get-Random -Min 40 -Max 95) / 100.0, 2)
    $tokOut = [int]($tokIn * $ratio)
    $reqId  = [System.Guid]::NewGuid().ToString("N").Substring(0, 12)
    $ts     = (Get-Date).AddSeconds(-($i * 2)).ToString("HH:mm:ss.fff")
    $status = if ($lvl -eq "WARN") { 503 } else { 200 }
    $layer  = Get-Random -Min 1 -Max 4

    $lines.Add("{""ts"":""$ts"",""level"":""$lvl"",""service"":""$svc"",""request_id"":""$reqId"",""method"":""$method"",""path"":""$path"",""user"":""$user"",""duration_ms"":$dur,""tokens_in"":$tokIn,""tokens_out"":$tokOut,""ratio"":$ratio,""layer"":$layer,""status"":$status}")
}

# Stack trace block (realistic error scenario)
$lines.Add("")
$lines.Add("2026-04-11T20:40:00.123Z ERROR compression-daemon thread 'tokio-runtime-worker' panicked:")
$lines.Add("called Result::unwrap() on an Err value: reqwest::Error { kind: Connect, source: hyper::Error(Connect,")
$lines.Add("  ConnectError(tcp connect error, Os { code: 111, kind: ConnectionRefused, message: Connection refused }))")
$lines.Add("   0: std::panicking::begin_panic")
$lines.Add("   1: ntk::compressor::layer3_llamacpp::LlamaCppBackend::compress")
$lines.Add("      at src/compressor/layer3_llamacpp.rs:142:14")
$lines.Add("   2: ntk::compressor::layer3_backend::BackendKind::compress")
$lines.Add("      at src/compressor/layer3_backend.rs:110:57")
$lines.Add("   3: ntk::server::compress_handler")
$lines.Add("      at src/server.rs:87:20")
$lines.Add("   4: axum::handler::Handler::call")
$lines.Add("      at .cargo/registry/src/axum-0.7.9/src/handler/mod.rs:115:42")
$lines.Add("note: run with RUST_BACKTRACE=1 to display a full backtrace")
$lines.Add("")

# Metrics summary block
$lines.Add("=== Session Metrics (last 60s) ===")
$lines.Add("total_compressions : 847")
$lines.Add("tokens_in_total    : 2_341_892")
$lines.Add("tokens_out_total   : 412_334")
$lines.Add("avg_ratio          : 0.824")
$lines.Add("layer_distribution : L1=42pct L2=35pct L3=23pct")
$lines.Add("p50_latency_ms     : 12")
$lines.Add("p95_latency_ms     : 387")
$lines.Add("p99_latency_ms     : 892")
$lines.Add("errors_total       : 3")
$lines.Add("fallbacks_l1       : 3")
$lines.Add("")
$lines.Add("=== Layer 3 Backend Status ===")
$lines.Add("provider           : ollama")
$lines.Add("model              : phi3:mini")
$lines.Add("ollama_url         : http://localhost:11434")
$lines.Add("status             : DEGRADED -- connection refused (auto-fallback to L1+L2)")
$lines.Add("last_success_at    : 2026-04-11T19:58:12Z")
$lines.Add("")

$lines -join "`n"
