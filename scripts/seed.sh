#!/usr/bin/env bash
# Seed the Datem data store with sample metrics.
#
# Usage:
#   ./scripts/seed.sh <api_url> [api_key]
#
# Examples:
#   ./scripts/seed.sh http://localhost:3000
#   ./scripts/seed.sh http://localhost:3000 dev-api-key
#   ./scripts/seed.sh https://api.acme.com my-secret-key

set -euo pipefail

API_URL="${1:?Usage: seed.sh <api_url> [api_key]}"
API_URL="${API_URL%/}"   # strip trailing slash
API_KEY="${2:-dev-api-key}"

# ── Colours ───────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'
BOLD='\033[1m'; NC='\033[0m'

# ── Helpers ───────────────────────────────────────────────────────────────────
api_post() {
    local path="$1" body="$2"
    curl -s -w "\n%{http_code}" \
        -X POST "$API_URL$path" \
        -H "Authorization: Bearer $API_KEY" \
        -H "Content-Type: application/json" \
        -d "$body"
}

api_get() {
    local path="$1"
    curl -s -w "\n%{http_code}" \
        "$API_URL$path" \
        -H "Authorization: Bearer $API_KEY"
}

# Prints ✓ or ✗ and exits on unexpected status.
# Pass a space-separated list of acceptable codes as $3 (default "200 201").
check() {
    local label="$1" response="$2" ok_codes="${3:-200 201}"
    local body code
    body=$(echo "$response" | head -n -1)
    code=$(echo "$response" | tail -n 1)

    if echo "$ok_codes" | grep -qw "$code"; then
        if echo "$ok_codes" | grep -qw "409" && [ "$code" = "409" ]; then
            echo -e "  ${YELLOW}~${NC} $label (already exists)"
        else
            echo -e "  ${GREEN}✓${NC} $label ($code)"
        fi
    else
        echo -e "  ${RED}✗${NC} $label — got $code"
        echo "    $body"
        exit 1
    fi
}

# ── Main ──────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Datem seed${NC}  →  $API_URL"
echo "──────────────────────────────────────────────────"

# Health check
printf "Checking API... "
code=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/health")
if [ "$code" = "200" ]; then
    echo -e "${GREEN}up${NC}"
else
    echo -e "${RED}unreachable (HTTP $code)${NC}"
    exit 1
fi

# ── Metrics ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Metrics${NC}"

# id, display, aggregation
METRICS=(
    "api_calls|API Calls|sum"
    "llm_tokens|LLM Tokens|sum"
    "active_users|Active Users|unique_count"
    "storage_gb|Storage (GB)|max"
    "jobs_run|Jobs Run|count"
)

for entry in "${METRICS[@]}"; do
    IFS='|' read -r id display agg <<< "$entry"
    response=$(api_post /metrics \
        "{\"id\":\"$id\",\"display\":\"$display\",\"aggregation\":\"$agg\"}")
    check "$id ($agg)" "$response" "200 201 409"
done

# Verify
response=$(api_get /metrics)
code=$(echo "$response" | tail -n 1)
body=$(echo "$response" | head -n -1)
if [ "$code" = "200" ]; then
    # Count items — try jq, fall back to grep
    if command -v jq &>/dev/null; then
        count=$(echo "$body" | jq '.data | length')
    else
        count=$(echo "$body" | grep -o '"id"' | wc -l | tr -d ' ')
    fi
    echo -e "  ${GREEN}✓${NC} GET /metrics → $count metrics"
else
    echo -e "  ${RED}✗${NC} GET /metrics — HTTP $code"
fi

# ── Plans ─────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Plans${NC}"

response=$(api_post /plans '{
  "id":       "starter-v1",
  "name":     "Starter",
  "currency": "usd",
  "interval": "monthly",
  "charges": [
    { "model": "flat",    "amount": 900,  "display": "Starter base fee" },
    { "model": "package", "metric": "api_calls",  "package_size": 1000,    "package_price": 10  },
    { "model": "package", "metric": "llm_tokens", "package_size": 1000000, "package_price": 200 }
  ]
}')
check "starter-v1" "$response" "200 201 409"

response=$(api_post /plans '{
  "id":       "pro-v1",
  "name":     "Pro",
  "currency": "usd",
  "interval": "monthly",
  "charges": [
    { "model": "flat",     "amount": 4900, "display": "Pro base fee" },
    { "model": "package",  "metric": "llm_tokens",   "package_size": 1000000, "package_price": 150 },
    { "model": "package",  "metric": "api_calls",    "package_size": 1000,    "package_price": 5   },
    { "model": "per_unit", "metric": "active_users", "unit_price": 25 }
  ]
}')
check "pro-v1" "$response" "200 201 409"

# Verify
response=$(api_get /plans)
code=$(echo "$response" | tail -n 1)
body=$(echo "$response" | head -n -1)
if [ "$code" = "200" ]; then
    if command -v jq &>/dev/null; then
        count=$(echo "$body" | jq '.data | length')
    else
        count=$(echo "$body" | grep -o '"id"' | wc -l | tr -d ' ')
    fi
    echo -e "  ${GREEN}✓${NC} GET /plans → $count plans"
else
    echo -e "  ${RED}✗${NC} GET /plans — HTTP $code"
fi

# ── Customers ─────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Customers${NC}"

CUSTOMERS=(
    "cust_acme|Acme Corp|billing@acme.com"
    "cust_globex|Globex Inc|billing@globex.com"
    "cust_initech|Initech|billing@initech.com"
)

for entry in "${CUSTOMERS[@]}"; do
    IFS='|' read -r id name email <<< "$entry"
    response=$(api_post /customers \
        "{\"id\":\"$id\",\"name\":\"$name\",\"email\":\"$email\"}")
    check "$id" "$response" "200 201 409"
done

response=$(api_get /customers)
code=$(echo "$response" | tail -n 1)
body=$(echo "$response" | head -n -1)
if [ "$code" = "200" ]; then
    if command -v jq &>/dev/null; then
        count=$(echo "$body" | jq '.data | length')
    else
        count=$(echo "$body" | grep -o '"id"' | wc -l | tr -d ' ')
    fi
    echo -e "  ${GREEN}✓${NC} GET /customers → $count customers"
else
    echo -e "  ${RED}✗${NC} GET /customers — HTTP $code"
fi

# ── Subscriptions ─────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Subscriptions${NC}"

SUBSCRIPTIONS=(
    "sub_acme_starter|cust_acme|starter-v1"
    "sub_globex_pro|cust_globex|pro-v1"
    "sub_initech_starter|cust_initech|starter-v1"
)

for entry in "${SUBSCRIPTIONS[@]}"; do
    IFS='|' read -r id customer_id plan_id <<< "$entry"
    response=$(api_post /subscriptions \
        "{\"id\":\"$id\",\"customer_id\":\"$customer_id\",\"plan_id\":\"$plan_id\"}")
    check "$id" "$response" "200 201 409"
done

response=$(api_get /customers/cust_acme/subscriptions)
code=$(echo "$response" | tail -n 1)
body=$(echo "$response" | head -n -1)
if [ "$code" = "200" ]; then
    if command -v jq &>/dev/null; then
        count=$(echo "$body" | jq '.data | length')
    else
        count=$(echo "$body" | grep -o '"id"' | wc -l | tr -d ' ')
    fi
    echo -e "  ${GREEN}✓${NC} GET /customers/cust_acme/subscriptions → $count subscription(s)"
else
    echo -e "  ${RED}✗${NC} GET /customers/cust_acme/subscriptions — HTTP $code"
fi

# ── Events (sample usage) ─────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Events${NC}"

NOW_US=$(date +%s)000000

# Batch ingest for cust_acme
response=$(api_post /ingest/batch "{
  \"events\": [
    {\"event_id\":\"seed-acme-001\",\"customer_id\":\"cust_acme\",\"metric\":\"api_calls\",\"quantity\":1,\"timestamp\":$NOW_US},
    {\"event_id\":\"seed-acme-002\",\"customer_id\":\"cust_acme\",\"metric\":\"llm_tokens\",\"quantity\":4096,\"timestamp\":$NOW_US},
    {\"event_id\":\"seed-acme-003\",\"customer_id\":\"cust_acme\",\"metric\":\"api_calls\",\"quantity\":1,\"timestamp\":$NOW_US},
    {\"event_id\":\"seed-acme-004\",\"customer_id\":\"cust_acme\",\"metric\":\"active_users\",\"quantity\":1,\"timestamp\":$NOW_US},
    {\"event_id\":\"seed-globex-001\",\"customer_id\":\"cust_globex\",\"metric\":\"llm_tokens\",\"quantity\":8192,\"timestamp\":$NOW_US},
    {\"event_id\":\"seed-globex-002\",\"customer_id\":\"cust_globex\",\"metric\":\"api_calls\",\"quantity\":1,\"timestamp\":$NOW_US}
  ]
}")
code=$(echo "$response" | tail -n 1)
body=$(echo "$response" | head -n -1)
if [ "$code" = "202" ]; then
    if command -v jq &>/dev/null; then
        accepted=$(echo "$body" | jq '.accepted')
        rejected=$(echo "$body" | jq '.rejected')
    else
        accepted="?"
        rejected="?"
    fi
    echo -e "  ${GREEN}✓${NC} POST /ingest/batch → accepted=$accepted rejected=$rejected"
else
    echo -e "  ${RED}✗${NC} POST /ingest/batch — HTTP $code"
    echo "    $body"
fi

# Single event
response=$(api_post /ingest \
    "{\"event_id\":\"seed-initech-001\",\"customer_id\":\"cust_initech\",\"metric\":\"api_calls\",\"quantity\":1,\"timestamp\":$NOW_US}")
check "single event" "$response" "202"

# ── Future resources (add sections here as steps are implemented) ─────────────

echo ""
echo "──────────────────────────────────────────────────"
echo -e "${GREEN}${BOLD}Seed complete${NC}"
echo ""
