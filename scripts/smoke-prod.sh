#!/bin/bash
# Post-deploy smoke test for ethpayserver.
#
# Adapted from picoclaw/smoke-tests/ethpayserver-testnet.sh for use in CI
# post-deploy verification. Hits the public surface of the target deployment
# (health probes, invoice create/read, checkout page).
#
# Required env vars:
#   SMOKE_BASE_URL   — base URL of the deployed instance
#   SMOKE_API_KEY    — API key with invoice create/read permissions
#   SMOKE_STORE_ID   — store UUID the API key is scoped to
#
# Optional:
#   CHEF_BOT_TOKEN / CHEF_ALLOWED_USERS — Telegram alerting
#
# Exit 0 = all green. Exit 1 = at least one check failed.

set -u
set -o pipefail

: "${SMOKE_BASE_URL:?SMOKE_BASE_URL required — set to the deployed instance URL}"
: "${SMOKE_API_KEY:?SMOKE_API_KEY required — create a smoke-test API key}"
: "${SMOKE_STORE_ID:?SMOKE_STORE_ID required — set to the smoke-test store UUID}"
: "${CHEF_BOT_TOKEN:=}"
: "${CHEF_ALLOWED_USERS:=}"

FAILED=()
PASSED=()
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

log()  { printf '[%s] %s\n' "$TS" "$*"; }
fail() { log "FAIL: $*"; FAILED+=("$*"); }
pass() { log "PASS: $*"; PASSED+=("$*"); }

# ---------------------------------------------------------------------------
# 1. /health/live — process is up
# ---------------------------------------------------------------------------

check_live() {
  local code
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 "$SMOKE_BASE_URL/health/live" || echo 000)
  [[ "$code" == "200" ]] && pass "/health/live ($code)" || fail "/health/live ($code)"
}

# ---------------------------------------------------------------------------
# 2. /health/ready — DB + Redis + RPC chains reachable
# ---------------------------------------------------------------------------

check_ready() {
  local code
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 "$SMOKE_BASE_URL/health/ready" || echo 000)
  [[ "$code" == "200" ]] && pass "/health/ready ($code)" || fail "/health/ready ($code)"
}

# ---------------------------------------------------------------------------
# 3. /health/deep — all dependencies healthy
# ---------------------------------------------------------------------------

check_deep() {
  local code body
  body=$(curl -sS --max-time 10 "$SMOKE_BASE_URL/health/deep" 2>&1)
  code=$?
  if [[ $code -ne 0 ]]; then
    fail "/health/deep (curl error $code)"
    return
  fi
  # Check that postgres and redis are ok
  local pg_status redis_status
  pg_status=$(echo "$body" | python3 -c "import json,sys; print(json.load(sys.stdin)['postgres']['status'])" 2>/dev/null || echo "error")
  redis_status=$(echo "$body" | python3 -c "import json,sys; print(json.load(sys.stdin)['redis']['status'])" 2>/dev/null || echo "error")
  if [[ "$pg_status" == "ok" ]]; then
    pass "/health/deep postgres=$pg_status"
  else
    fail "/health/deep postgres=$pg_status"
  fi
  if [[ "$redis_status" == "ok" ]]; then
    pass "/health/deep redis=$redis_status"
  else
    fail "/health/deep redis=$redis_status"
  fi
}

# ---------------------------------------------------------------------------
# 4. Create an invoice via API key, then fetch it back
# ---------------------------------------------------------------------------

check_invoice_lifecycle() {
  local body resp inv_id code
  body=$(cat <<JSON
{"store_id":"$SMOKE_STORE_ID","amount":"1.00","currency":"USD","description":"smoke-test $TS"}
JSON
  )
  resp=$(curl -sS --max-time 15 -X POST \
    -H "Authorization: Bearer $SMOKE_API_KEY" \
    -H "Content-Type: application/json" \
    -d "$body" \
    "$SMOKE_BASE_URL/api/invoices" 2>&1)
  inv_id=$(echo "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('id',''))" 2>/dev/null || true)
  if [[ -z "$inv_id" ]]; then
    fail "POST /api/invoices — no id in response (${resp:0:200})"
    return
  fi
  pass "POST /api/invoices -> $inv_id"

  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -H "Authorization: Bearer $SMOKE_API_KEY" \
    "$SMOKE_BASE_URL/api/invoices/$inv_id" || echo 000)
  [[ "$code" == "200" ]] && pass "GET /api/invoices/$inv_id ($code)" \
    || fail "GET /api/invoices/$inv_id ($code)"

  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    "$SMOKE_BASE_URL/checkout/$inv_id" || echo 000)
  [[ "$code" == "200" ]] && pass "GET /checkout/$inv_id ($code)" \
    || fail "GET /checkout/$inv_id ($code)"
}

# ---------------------------------------------------------------------------
# 5. Alerting
# ---------------------------------------------------------------------------

telegram_alert() {
  [[ -z "$CHEF_BOT_TOKEN" || -z "$CHEF_ALLOWED_USERS" ]] && return
  local chat_id="${CHEF_ALLOWED_USERS%%,*}"
  local text="$1"
  curl -sS --max-time 10 -X POST \
    -d "chat_id=$chat_id" \
    --data-urlencode "text=$text" \
    "https://api.telegram.org/bot$CHEF_BOT_TOKEN/sendMessage" >/dev/null 2>&1 || true
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

check_live
check_ready
check_deep
check_invoice_lifecycle

if [[ ${#FAILED[@]} -gt 0 ]]; then
  msg="Smoke test FAILED on $SMOKE_BASE_URL (${#FAILED[@]} of $(( ${#PASSED[@]} + ${#FAILED[@]} )))
$(printf '  - %s\n' "${FAILED[@]}")
passed: ${#PASSED[@]}"
  telegram_alert "$msg"
  log "SUMMARY: ${#FAILED[@]} failed, ${#PASSED[@]} passed"
  exit 1
fi

log "SUMMARY: all ${#PASSED[@]} checks passed"
exit 0
