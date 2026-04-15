# NTK PostToolUse hook — PowerShell version for Windows.
# Reads Claude Code hook JSON from stdin, sends the Bash tool output
# to the NTK daemon for compression, and prints the result via stdout.
#
# Installed to: ~/.ntk/bin/ntk-hook.ps1
# Registered in: ~/.claude/settings.json  (hooks.PostToolUse)

param()

$ErrorActionPreference = 'Stop'

$NtkDaemonUrl = if ($env:NTK_DAEMON_URL) { $env:NTK_DAEMON_URL } else { "http://127.0.0.1:8765" }
$MinChars     = 500
# 65 s = slightly more than model.timeout_ms (60 s) so the daemon can fall back
# to L1+L2 before this request times out. Keep below Claude Code's own hook timeout.
$TimeoutSecs  = 65

# Read stdin — use $input pipeline variable (works for both piped and redirected stdin).
# [Console]::In.ReadToEnd() fails when PowerShell is launched as a subprocess with
# redirected stdin; $input correctly handles both interactive and subprocess contexts.
$input = $input -join "`n"
if (-not $input) {
    # Fallback: read line by line via [Console]::In for compatibility with some callers
    $lines = @()
    try {
        while ($true) {
            $line = [Console]::In.ReadLine()
            if ($null -eq $line) { break }
            $lines += $line
        }
    } catch {}
    $input = $lines -join "`n"
}

# Parse hook JSON.
try {
    $hook = $input | ConvertFrom-Json
} catch {
    exit 0
}

# Only process Bash tool results.
if ($hook.tool_name -ne "Bash") {
    exit 0
}

$output  = if ($hook.tool_response.output) { $hook.tool_response.output } else { "" }
$command = if ($hook.tool_input.command)   { $hook.tool_input.command }   else { "" }
$cwd     = if ($hook.cwd)                  { $hook.cwd }                  else { "" }

# Skip short outputs.
if ($output.Length -lt $MinChars) {
    exit 0
}

# Build JSON payload.
$payload = @{
    output  = $output
    command = $command
    cwd     = $cwd
} | ConvertTo-Json -Compress -Depth 5

# POST to daemon via System.Net.WebRequest (synchronous, no async, works in PS5 subprocesses).
try {
    $bytes   = [System.Text.Encoding]::UTF8.GetBytes($payload)
    $req     = [System.Net.WebRequest]::Create("${NtkDaemonUrl}/compress")
    $req.Method      = 'POST'
    $req.ContentType = 'application/json'
    $req.Timeout     = $TimeoutSecs * 1000
    $req.ContentLength = $bytes.Length
    $reqStream = $req.GetRequestStream()
    $reqStream.Write($bytes, 0, $bytes.Length)
    $reqStream.Close()
    $resp   = $req.GetResponse()
    $reader = New-Object System.IO.StreamReader($resp.GetResponseStream())
    $responseJson = $reader.ReadToEnd()
    $reader.Close()
    $resp.Close()
} catch {
    exit 0
}

$response = try { $responseJson | ConvertFrom-Json } catch { exit 0 }

if (-not $response -or -not $response.compressed) {
    exit 0
}

# Build additionalContext string.
$ratio_pct = [int]($response.ratio * 100)
$ctx = $response.compressed
$ctx += "`n[NTK L$($response.layer): $($response.tokens_before)->$($response.tokens_after) tokens ($ratio_pct% saved)]"

# Emit hook output JSON for Claude Code.
@{
    hookSpecificOutput = @{
        hookEventName     = "PostToolUse"
        additionalContext = $ctx
    }
} | ConvertTo-Json -Compress -Depth 5

exit 0
