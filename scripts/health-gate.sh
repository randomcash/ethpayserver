#!/bin/bash
# Post-deploy health gate for ethpayserver.
#
# Polls /health/deep every 5 seconds for up to HEALTH_TIMEOUT seconds.
# Succeeds when:
#   1. The endpoint returns 200
#   2. build_sha matches EXPECTED_SHA (if set)
#   3. Postgres and Redis report "ok"
#   4. All RPC chains report "ok" (no chain in error/disconnected state)
#
# If the gate does not pass within the timeout, exit 1 — the previous
# container image stays live (Docker Compose health-check prevents cutover).
#
# Required env vars:
#   HEALTH_URL       — full URL to the /health/deep endpoint
#
# Optional:
#   HEALTH_TIMEOUT   — seconds to poll (default: 60)
#   EXPECTED_SHA     — expected build_sha in the response

set -u
set -o pipefail

: "${HEALTH_URL:?HEALTH_URL required — e.g. https://api.random.cash/health/deep}"
: "${HEALTH_TIMEOUT:=60}"
: "${EXPECTED_SHA:=}"

INTERVAL=5
ELAPSED=0

log() { printf '[health-gate] %s\n' "$*"; }

while [[ $ELAPSED -lt $HEALTH_TIMEOUT ]]; do
  BODY=$(curl -sS --max-time 10 "$HEALTH_URL" 2>&1) || {
    log "curl failed (elapsed ${ELAPSED}s), retrying..."
    sleep $INTERVAL
    ELAPSED=$((ELAPSED + INTERVAL))
    continue
  }

  # Parse response fields
  BUILD_SHA=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin).get('build_sha',''))" 2>/dev/null || echo "")
  PG_STATUS=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin)['postgres']['status'])" 2>/dev/null || echo "error")
  REDIS_STATUS=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin)['redis']['status'])" 2>/dev/null || echo "error")
  MONITOR_FRESH=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin)['monitor']['data_fresh'])" 2>/dev/null || echo "False")

  # Check RPC chains — all must be "ok"
  RPC_BAD=$(echo "$BODY" | python3 -c "
import json, sys
data = json.load(sys.stdin)
bad = [k for k, v in data.get('rpcs', {}).items() if v.get('status') != 'ok']
print(','.join(bad) if bad else '')
" 2>/dev/null || echo "unknown")

  # Check build SHA if expected
  if [[ -n "$EXPECTED_SHA" && "$BUILD_SHA" != "$EXPECTED_SHA" ]]; then
    log "SHA mismatch: got=$BUILD_SHA expected=$EXPECTED_SHA (elapsed ${ELAPSED}s)"
    sleep $INTERVAL
    ELAPSED=$((ELAPSED + INTERVAL))
    continue
  fi

  # Check all dependencies
  if [[ "$PG_STATUS" != "ok" ]]; then
    log "postgres=$PG_STATUS (elapsed ${ELAPSED}s)"
    sleep $INTERVAL
    ELAPSED=$((ELAPSED + INTERVAL))
    continue
  fi

  if [[ "$REDIS_STATUS" != "ok" ]]; then
    log "redis=$REDIS_STATUS (elapsed ${ELAPSED}s)"
    sleep $INTERVAL
    ELAPSED=$((ELAPSED + INTERVAL))
    continue
  fi

  if [[ -n "$RPC_BAD" ]]; then
    log "unhealthy chains: $RPC_BAD (elapsed ${ELAPSED}s)"
    sleep $INTERVAL
    ELAPSED=$((ELAPSED + INTERVAL))
    continue
  fi

  # All checks passed
  log "HEALTHY — sha=$BUILD_SHA pg=ok redis=ok rpcs=all_ok (${ELAPSED}s)"
  exit 0
done

log "TIMEOUT after ${HEALTH_TIMEOUT}s — deploy health gate FAILED"
log "Last response: pg=$PG_STATUS redis=$REDIS_STATUS rpc_bad=$RPC_BAD sha=$BUILD_SHA"
exit 1
