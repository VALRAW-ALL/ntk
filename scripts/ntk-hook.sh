#!/usr/bin/env bash
# NTK PostToolUse hook — reads Claude Code hook JSON from stdin,
# sends the Bash tool output to the NTK daemon for compression,
# and prints the result back to Claude Code via stdout.
#
# Installed to: ~/.ntk/bin/ntk-hook.sh
# Registered in: ~/.claude/settings.json  (hooks.PostToolUse)

set -euo pipefail

NTK_DAEMON_URL="${NTK_DAEMON_URL:-http://127.0.0.1:8765}"
MIN_CHARS=500       # skip compression for short outputs
# 305 s = just over the daemon's default model.timeout_ms (300 s / 5 min) so
# the daemon has time to fall back to L1+L2 before this request aborts.
TIMEOUT_SECS=305

# Read full stdin into a variable.
input=$(cat)

# Extract fields from the hook JSON.
tool_name=$(printf '%s' "$input" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_name',''))" 2>/dev/null || echo "")
output=$(printf '%s' "$input" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_response',{}).get('output',''))" 2>/dev/null || echo "")
command=$(printf '%s' "$input" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command',''))" 2>/dev/null || echo "")
cwd=$(printf '%s' "$input" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('cwd',''))" 2>/dev/null || echo "")
transcript_path=$(printf '%s' "$input" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('transcript_path',''))" 2>/dev/null || echo "")

# Only process Bash tool results.
if [ "$tool_name" != "Bash" ]; then
    exit 0
fi

# Skip short outputs — not worth the daemon roundtrip.
char_count=${#output}
if [ "$char_count" -lt "$MIN_CHARS" ]; then
    exit 0
fi

# Build JSON payload. transcript_path enables Layer 4 context injection on
# the daemon side — it reads the Claude Code session JSONL to bias L3 toward
# information relevant to the user's current intent.
payload=$(python3 -c "
import json, sys
print(json.dumps({
    'output': sys.argv[1],
    'command': sys.argv[2],
    'cwd': sys.argv[3],
    'transcript_path': sys.argv[4],
}))
" "$output" "$command" "$cwd" "$transcript_path" 2>/dev/null)

if [ -z "$payload" ]; then
    exit 0
fi

# POST to daemon. On any error, exit 0 so Claude Code uses the original output.
response=$(curl -s --max-time "$TIMEOUT_SECS" \
    -X POST "${NTK_DAEMON_URL}/compress" \
    -H "Content-Type: application/json" \
    -d "$payload" 2>/dev/null) || exit 0

if [ -z "$response" ]; then
    exit 0
fi

# Extract compressed output and stats from the daemon response.
# Daemon returns: { "compressed": "...", "layer": N, "tokens_before": N, "tokens_after": N, "ratio": 0.xx }
compressed=$(printf '%s' "$response" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('compressed',''))" 2>/dev/null || echo "")
layer=$(printf '%s' "$response"      | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('layer',0))" 2>/dev/null || echo "0")
tok_in=$(printf '%s' "$response"     | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tokens_before',0))" 2>/dev/null || echo "0")
tok_out=$(printf '%s' "$response"    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tokens_after',0))" 2>/dev/null || echo "0")
ratio=$(printf '%s' "$response"      | python3 -c "import sys,json; d=json.load(sys.stdin); print(int(d.get('ratio',0)*100))" 2>/dev/null || echo "0")

if [ -z "$compressed" ]; then
    exit 0
fi

ntk_note="[NTK L${layer}: ${tok_in}->${tok_out} tokens (${ratio}% saved)]"

# Emit the hook output JSON for Claude Code.
python3 -c "
import json, sys
print(json.dumps({
    'hookSpecificOutput': {
        'hookEventName': 'PostToolUse',
        'additionalContext': sys.argv[1],
    }
}))
" "${compressed}
${ntk_note}" 2>/dev/null

exit 0
