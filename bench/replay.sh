#!/bin/sh
# NTK microbench replay — POSTs each bench/fixtures/*.txt against the
# daemon's /compress endpoint and writes microbench.csv.
#
# Requirements: curl, jq. Daemon running on DAEMON_URL (default
# http://127.0.0.1:8765). Set NTK_LOG_COMPRESSIONS=1 before starting the
# daemon if you also want the raw input + per-layer outputs persisted.

set -eu

DAEMON_URL="${DAEMON_URL:-http://127.0.0.1:8765}"
HERE="$(cd "$(dirname "$0")" && pwd)"
FIXTURES="$HERE/fixtures"
OUT_CSV="${OUT_CSV:-$HERE/microbench.csv}"
TIMEOUT_SEC="${TIMEOUT_SEC:-300}"

# Sanity: daemon reachable?
if ! curl -sf --max-time 3 "$DAEMON_URL/health" > /dev/null; then
    echo "daemon unreachable at $DAEMON_URL — start it with \`ntk start\`" >&2
    exit 1
fi
echo "  daemon OK at $DAEMON_URL"

# Check required tools
for tool in curl jq; do
    if ! command -v "$tool" > /dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 1
    fi
done

echo 'fixture,bytes_in,tokens_before,tokens_after_l1,tokens_after_l2,tokens_after_l3,tokens_after,layer_used,ratio,latency_ms_total,latency_ms_l1,latency_ms_l2,latency_ms_l3,http_status,error' > "$OUT_CSV"

printf '\n%-40s %10s %10s %6s %8s %8s\n' 'fixture' 'before' 'after' 'L' 'ratio' 'ms'
printf -- '-%.0s' $(seq 1 88); printf '\n'

for fx in "$FIXTURES"/*.txt; do
    name=$(basename "$fx" .txt)
    bytes_in=$(wc -c < "$fx" | tr -d ' ')
    meta="$FIXTURES/$name.meta.json"
    cmd='unknown'
    if [ -f "$meta" ]; then
        cmd=$(jq -r '.command // "unknown"' "$meta")
    fi

    payload=$(jq -n --rawfile out "$fx" --arg cmd "$cmd" \
        '{output: $out, command: $cmd, cwd: "bench"}')

    t0=$(date +%s%N 2>/dev/null || date +%s)
    tmp_resp=$(mktemp)
    status=$(curl -s -o "$tmp_resp" -w '%{http_code}' \
        --max-time "$TIMEOUT_SEC" \
        -X POST \
        -H 'Content-Type: application/json' \
        --data-binary @- \
        "$DAEMON_URL/compress" <<EOF
$payload
EOF
    ) || status="000"
    t1=$(date +%s%N 2>/dev/null || date +%s)
    # Normalise elapsed to ms. $(( )) only needs integer arithmetic.
    case "$t0" in
        *[!0-9]*) elapsed_ms=0 ;;
        *)
            case "$t1" in
                *[!0-9]*) elapsed_ms=0 ;;
                *) elapsed_ms=$(( (t1 - t0) / 1000000 )) ;;
            esac
            ;;
    esac

    body=$(cat "$tmp_resp")
    rm -f "$tmp_resp"

    if [ "$status" = "200" ]; then
        tBefore=$(echo "$body" | jq -r '.tokens_before // ""')
        tL1=$(echo "$body"     | jq -r '.tokens_after_l1 // ""')
        tL2=$(echo "$body"     | jq -r '.tokens_after_l2 // ""')
        tL3=$(echo "$body"     | jq -r '.tokens_after_l3 // ""')
        tAfter=$(echo "$body"  | jq -r '.tokens_after // ""')
        layer=$(echo "$body"   | jq -r '.layer // ""')
        ratio=$(echo "$body"   | jq -r '.ratio // ""')
        msL1=$(echo "$body"    | jq -r '.latency_ms.l1 // ""')
        msL2=$(echo "$body"    | jq -r '.latency_ms.l2 // ""')
        msL3=$(echo "$body"    | jq -r '.latency_ms.l3 // ""')
        err=''
    else
        tBefore=''; tL1=''; tL2=''; tL3=''; tAfter=''; layer=''; ratio=''
        msL1=''; msL2=''; msL3=''
        err=$(echo "$body" | tr ',\n' ';; ' | head -c 200)
    fi

    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,"%s"\n' \
        "$name" "$bytes_in" "$tBefore" "$tL1" "$tL2" "$tL3" "$tAfter" \
        "$layer" "$ratio" "$elapsed_ms" "$msL1" "$msL2" "$msL3" \
        "$status" "$err" >> "$OUT_CSV"

    display_after=${tAfter:-ERR}
    display_ratio=${ratio:--}
    printf '%-40s %10s %10s %6s %8s %8s\n' \
        "$name" "${tBefore:-}" "$display_after" "${layer:-}" "$display_ratio" "$elapsed_ms"
done

echo ""
echo "wrote: $OUT_CSV"
