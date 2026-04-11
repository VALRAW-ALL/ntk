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
$TimeoutSecs  = 10

# Read stdin.
$input = [Console]::In.ReadToEnd()

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

# POST to daemon.
try {
    $response = Invoke-RestMethod `
        -Uri "${NtkDaemonUrl}/compress" `
        -Method Post `
        -ContentType "application/json" `
        -Body $payload `
        -TimeoutSec $TimeoutSecs
} catch {
    exit 0
}

if (-not $response -or -not $response.output) {
    exit 0
}

# Build additionalContext string.
$ctx = $response.output
if ($response.summary) {
    $ctx += "`n[NTK: $($response.summary)]"
}

# Emit hook output JSON for Claude Code.
@{
    hookSpecificOutput = @{
        hookEventName     = "PostToolUse"
        additionalContext = $ctx
    }
} | ConvertTo-Json -Compress -Depth 5

exit 0
