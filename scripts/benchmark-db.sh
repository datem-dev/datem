#!/usr/bin/env bash
# Run the Rust benchmark harness (bin/bench) through a standard suite of
# scenarios and print a combined results table.
#
# Precondition: run ./scripts/seed.sh <api_url> first to load sample data.
#
# Usage:
#   ./scripts/benchmark-db.sh <api_url> [api_key] [duration_secs]
#
# Examples:
#   ./scripts/benchmark-db.sh http://localhost:3000
#   ./scripts/benchmark-db.sh http://localhost:3000 dev-api-key 15

set -euo pipefail

API_URL="${1:?Usage: benchmark-db.sh <api_url> [api_key] [duration_secs]}"
API_URL="${API_URL%/}"
API_KEY="${2:-dev-api-key}"
DURATION="${3:-20}"

# ── Colours ───────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_BIN="$REPO_ROOT/target/release/bench"

TMPDIR_LOCAL=$(mktemp -d)
trap 'rm -rf "$TMPDIR_LOCAL"' EXIT

RESULTS_DIR="$REPO_ROOT/bench-results"
mkdir -p "$RESULTS_DIR"
RUN_STAMP=$(date +%Y%m%d-%H%M%S)
COMBINED_JSON="$RESULTS_DIR/$RUN_STAMP.json"

# ── Health check ──────────────────────────────────────────────────────────────
printf "Checking API... "
code=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/health")
if [ "$code" != "200" ]; then
    echo -e "${RED}unreachable (HTTP $code)${NC}"
    exit 1
fi
echo -e "${GREEN}up${NC}"

metric_code=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer $API_KEY" "$API_URL/metrics/api_calls")
if [ "$metric_code" != "200" ]; then
    echo -e "${YELLOW}Warning:${NC} metric 'api_calls' not found (HTTP $metric_code)."
    echo -e "  Run ${BOLD}./scripts/seed.sh $API_URL${NC} first to load test data."
    echo ""
fi

echo ""
echo -e "${BOLD}Datem benchmark harness${NC}"
echo "  URL:      $API_URL"
echo "  Duration: ${DURATION}s per scenario"
echo ""

# ── Build once, release mode ──────────────────────────────────────────────────
printf "Building bench binary (release)... "
(cd "$REPO_ROOT" && cargo build --release --bin bench --quiet)
echo -e "${GREEN}done${NC}"
echo ""

# ── Scenario suite ────────────────────────────────────────────────────────────
# label:workload:concurrency:batch_size
SCENARIOS=(
    "concurrency-1:mixed:1:100"
    "concurrency-10:mixed:10:100"
    "concurrency-50:mixed:50:100"
    "concurrency-100:mixed:100:100"
    "ingest-batch-1:ingest-batch:10:1"
    "ingest-batch-10:ingest-batch:10:10"
    "ingest-batch-100:ingest-batch:10:100"
    "ingest-batch-500:ingest-batch:10:500"
    "read-heavy:metrics:10:100"
    "write-heavy:ingest-one:10:100"
)

print_header() {
    printf "\n${BOLD}%-20s  %-20s  %8s  %8s  %8s  %10s  %6s${NC}\n" \
        "Scenario" "Endpoint" "P50 (ms)" "P95 (ms)" "P99 (ms)" "Req/s" "Errors"
    printf '%s\n' "$(printf '─%.0s' {1..90})"
}

print_header

RUN_RESULTS=()
for scenario in "${SCENARIOS[@]}"; do
    IFS=':' read -r label workload concurrency batch_size <<< "$scenario"

    out_file="$TMPDIR_LOCAL/$label.json"
    printf "  ${CYAN}%-20s${NC} running...\n" "$label" >&2

    "$BENCH_BIN" \
        --api-url "$API_URL" \
        --api-key "$API_KEY" \
        --workload "$workload" \
        --concurrency "$concurrency" \
        --duration-secs "$DURATION" \
        --batch-size "$batch_size" \
        --format json > "$out_file"

    RUN_RESULTS+=("$label:$out_file")

    jq -r --arg label "$label" '
        .[] | [$label, .label, (.p50_ms|tostring), (.p95_ms|tostring),
               (.p99_ms|tostring), (.req_per_sec|tostring), (.err|tostring)]
        | @tsv
    ' "$out_file" | while IFS=$'\t' read -r s ep p50 p95 p99 rps errs; do
        err_color="$NC"
        [ "$errs" != "0" ] && err_color="$RED"
        printf "%-20s  %-20s  %8.1f  %8.1f  %8.1f  %10.1f  ${err_color}%6s${NC}\n" \
            "$s" "$ep" "$p50" "$p95" "$p99" "$rps" "$errs"
    done
done

printf '%s\n' "$(printf '─%.0s' {1..90})"

# ── Combine into one JSON file for later diffing ──────────────────────────────
{
    echo "["
    first=true
    for entry in "${RUN_RESULTS[@]}"; do
        scenario_label="${entry%%:*}"
        file="${entry#*:}"
        $first || echo ","
        first=false
        jq --arg scenario "$scenario_label" '{scenario: $scenario, results: .}' "$file"
    done
    echo "]"
} | jq -s 'add' > "$COMBINED_JSON"

echo ""
echo -e "Combined results written to ${BOLD}$COMBINED_JSON${NC}"
