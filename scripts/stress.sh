#!/usr/bin/env bash
# Stress test the Datem API and report throughput + latency.
#
# Usage:
#   ./scripts/stress.sh <api_url> [api_key] [concurrency] [duration_secs]
#
# Examples:
#   ./scripts/stress.sh http://localhost:3000
#   ./scripts/stress.sh http://localhost:3000 dev-api-key 20 60
#   ./scripts/stress.sh https://api.acme.com my-key 50 120

set -euo pipefail

API_URL="${1:?Usage: stress.sh <api_url> [api_key] [concurrency] [duration_secs]}"
API_URL="${API_URL%/}"
API_KEY="${2:-dev-api-key}"
CONCURRENCY="${3:-10}"
DURATION="${4:-30}"

# ── Colours ───────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

TMPDIR_LOCAL=$(mktemp -d)
trap 'rm -rf "$TMPDIR_LOCAL"' EXIT

# ── Health check ──────────────────────────────────────────────────────────────
printf "Checking API... "
code=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/health")
if [ "$code" != "200" ]; then
    echo -e "${RED}unreachable (HTTP $code)${NC}"
    exit 1
fi
echo -e "${GREEN}up${NC}"

echo ""
echo -e "${BOLD}Datem stress test${NC}"
echo "  URL:         $API_URL"
echo "  Concurrency: $CONCURRENCY workers"
echo "  Duration:    ${DURATION}s per endpoint"
echo ""

# ── Latency sample (sequential, 50 requests) ─────────────────────────────────
# Returns p50 p95 p99 (in ms) via stdout, space-separated.
latency_stats() {
    local url="$1" method="${2:-GET}" body="${3:-}"
    local n=50
    local times_file="$TMPDIR_LOCAL/times_$$"

    for _ in $(seq 1 $n); do
        if [ "$method" = "POST" ] && [ -n "$body" ]; then
            curl -s -o /dev/null -w "%{time_total}\n" \
                -X POST \
                -H "Authorization: Bearer $API_KEY" \
                -H "Content-Type: application/json" \
                -d "$body" "$url"
        else
            curl -s -o /dev/null -w "%{time_total}\n" \
                -H "Authorization: Bearer $API_KEY" "$url"
        fi
    done > "$times_file"

    # Convert to ms and compute p50/p95/p99 with awk
    sort -n "$times_file" | awk -v n=$n '
        { t[NR] = $1 * 1000 }
        END {
            p50 = t[int(n * 0.50) + 1]
            p95 = t[int(n * 0.95) + 1]
            p99 = t[int(n * 0.99) + 1]
            printf "%.1f %.1f %.1f\n", p50, p95, p99
        }
    '
    rm -f "$times_file"
}

# ── Throughput test (parallel workers for DURATION seconds) ──────────────────
# Writes request count to a per-worker file; sums at the end.
throughput_test() {
    local label="$1" url="$2" method="${3:-GET}" body="${4:-}"
    local work_dir="$TMPDIR_LOCAL/tp_${label//\//_}"
    mkdir -p "$work_dir"

    local end_time=$(( $(date +%s) + DURATION ))

    worker() {
        local wid="$1" url="$2" method="$3" body="$4" end="$5" wdir="$6"
        local ok=0 err=0 seq=0
        while [ "$(date +%s)" -lt "$end" ]; do
            seq=$(( seq + 1 ))
            if [ "$method" = "POST" ] && [ -n "$body" ]; then
                # Replace event_id with a unique value per worker per request.
                local req_body
                req_body=$(echo "$body" | sed "s/\"event_id\":\"[^\"]*\"/\"event_id\":\"w${wid}-${seq}\"/" )
                code=$(curl -s -o /dev/null -w "%{http_code}" \
                    -X POST \
                    -H "Authorization: Bearer $API_KEY" \
                    -H "Content-Type: application/json" \
                    -d "$req_body" "$url")
            else
                code=$(curl -s -o /dev/null -w "%{http_code}" \
                    -H "Authorization: Bearer $API_KEY" "$url")
            fi
            if [[ "$code" =~ ^2 ]]; then
                ok=$(( ok + 1 ))
            else
                err=$(( err + 1 ))
            fi
        done
        echo "$ok $err" > "$wdir/$wid"
    }
    export -f worker

    # Spawn workers
    local pids=()
    for i in $(seq 1 "$CONCURRENCY"); do
        worker "$i" "$url" "$method" "$body" "$end_time" "$work_dir" &
        pids+=($!)
    done

    # Progress dots while running
    local remaining=$DURATION
    while [ $remaining -gt 0 ]; do
        printf "."
        sleep 1
        remaining=$(( remaining - 1 ))
    done

    wait "${pids[@]}" 2>/dev/null

    # Tally results
    local total_ok=0 total_err=0
    for f in "$work_dir"/*; do
        read -r ok err < "$f"
        total_ok=$(( total_ok + ok ))
        total_err=$(( total_err + err ))
    done

    local total=$(( total_ok + total_err ))
    local rps=$(( total / DURATION ))
    echo "$total_ok $total_err $rps"
}

# ── Run benchmarks ────────────────────────────────────────────────────────────
print_header() {
    printf "\n${BOLD}%-30s  %8s  %8s  %8s  %10s  %6s${NC}\n" \
        "Endpoint" "P50 (ms)" "P95 (ms)" "P99 (ms)" "Req/s" "Errors"
    printf '%s\n' "$(printf '─%.0s' {1..80})"
}

print_row() {
    local label="$1" p50="$2" p95="$3" p99="$4" rps="$5" errs="$6"
    local err_color="$NC"
    [ "$errs" -gt 0 ] && err_color="$RED"
    printf "%-30s  %8s  %8s  %8s  %10s  ${err_color}%6s${NC}\n" \
        "$label" "$p50" "$p95" "$p99" "$rps" "$errs"
}

run_bench() {
    local label="$1" url="$2" method="${3:-GET}" body="${4:-}"
    printf "  %-28s  latency..." "$label" >&2
    read -r p50 p95 p99 < <(latency_stats "$url" "$method" "$body")
    printf " throughput" >&2
    read -r ok err rps < <(throughput_test "$label" "$url" "$method" "$body")
    printf ".\n" >&2
    print_row "$label" "$p50" "$p95" "$p99" "$rps" "$err"
}

# ── Check seed data ───────────────────────────────────────────────────────────
seed_ok=true
metric_code=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer $API_KEY" "$API_URL/metrics/api_calls")
if [ "$metric_code" != "200" ]; then
    seed_ok=false
fi
if ! $seed_ok; then
    echo -e "${YELLOW}Warning:${NC} metric 'api_calls' not found (HTTP $metric_code)." >&2
    echo -e "  Run ${BOLD}./scripts/seed.sh $API_URL${NC} first to load test data." >&2
    echo "" >&2
fi

print_header

printf "Running benchmarks (this takes ~$((DURATION * 4))s)...\n" >&2

run_bench "GET /health"           "$API_URL/health"
run_bench "GET /metrics"          "$API_URL/metrics"
run_bench "GET /metrics/api_calls" "$API_URL/metrics/api_calls"

# Generate a unique event_id prefix per run; workers append their worker ID.
RUN_ID="stress-$(date +%s)"

INGEST_PAYLOAD='{
  "event_id":"'"$RUN_ID"'-lat",
  "customer_id":"cust_acme",
  "metric":"api_calls",
  "quantity":1,
  "timestamp":'"$(date +%s)"'000000
}'

run_bench "POST /ingest" "$API_URL/ingest" POST "$INGEST_PAYLOAD"

printf '%s\n' "$(printf '─%.0s' {1..80})"
echo ""
