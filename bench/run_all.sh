#!/bin/sh
# NTK benchmark orchestrator — Unix/macOS version of run_all.ps1.
# Generates fixtures (if missing), restarts the daemon with compression
# logging, runs replay.sh, and prints results.
#
# Usage:
#   NTK_BIN=/usr/local/bin/ntk bash bench/run_all.sh
#
# Requires: curl, jq, a POSIX shell. Daemon restart step uses `ntk` on
# PATH unless overridden via NTK_BIN.

set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
NTK_BIN="${NTK_BIN:-ntk}"
TIMEOUT_SEC="${TIMEOUT_SEC:-300}"
SKIP_DAEMON="${SKIP_DAEMON:-0}"

# 1. Ensure fixtures exist — they're checked into git, but regenerate if a
#    fresh clone happens to be missing them.
if [ "$(find "$HERE/fixtures" -name '*.txt' 2>/dev/null | wc -l | tr -d ' ')" -lt 8 ]; then
    echo '==> Fixtures missing. The PowerShell generator must be run under pwsh;'
    echo '    on Unix it is easier to check them in — which we do.  Skipping.'
fi

# 2. Restart daemon with logging enabled.
if [ "$SKIP_DAEMON" != "1" ]; then
    echo '==> Stopping any running daemon...'
    "$NTK_BIN" stop 2>/dev/null || true

    echo '==> Starting daemon with NTK_LOG_COMPRESSIONS=1 in background...'
    NTK_LOG_COMPRESSIONS=1 "$NTK_BIN" start > /tmp/ntk_daemon.log 2>&1 &

    # Wait up to 10 s for /health to respond.
    up=0
    i=0
    while [ "$i" -lt 10 ]; do
        sleep 1
        if curl -sf --max-time 2 http://127.0.0.1:8765/health > /dev/null 2>&1; then
            up=1
            break
        fi
        i=$((i + 1))
    done
    if [ "$up" != "1" ]; then
        echo 'daemon did not come up in 10s — check `ntk start` manually' >&2
        exit 1
    fi
fi

# 3. Replay fixtures
echo '==> Running microbench (this may take several minutes on CPU L3)...'
TIMEOUT_SEC="$TIMEOUT_SEC" bash "$HERE/replay.sh"

# 4. Report
echo ''
echo '==> Reports:'
echo "    microbench.csv : $HERE/microbench.csv"
echo "    logs           : $HOME/.ntk/logs"
echo ""
echo "    Generate the markdown report (requires pwsh for now):"
echo "      pwsh $HERE/report.ps1"
