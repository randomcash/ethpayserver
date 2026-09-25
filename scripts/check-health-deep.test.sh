#!/usr/bin/env bash
# Proves check-health-deep.sh goes red on the cases it claims to, against a
# real HTTP server on localhost - not a comment asserting someone ran it once.
#
# The failure mode this guards against is a check that stays green through the
# outage it was written to catch (monitor.data_fresh false, an RPC gone quiet,
# a stalled last_block, or its own state-tracking erroring) - so "it passed on
# a healthy body" is not evidence it works.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-health-deep.sh"
TMP="$(mktemp -d)"
STATUS_FILE="$TMP/status"
BODY_FILE="$TMP/body"
STATE_FILE="$TMP/state.json"
CHECKIN_LOG="$TMP/checkin.log"
SERVER_PID=""

cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
  rm -rf "$TMP"
}
trap cleanup EXIT

healthy_body() {
  cat <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"1":{"status":"ok","last_block":100}}}
JSON
}

echo 200 > "$STATUS_FILE"
healthy_body > "$BODY_FILE"

cat > "$TMP/server.py" <<'PYEOF'
import http.server, socketserver, sys

port, status_file, body_file, checkin_log = sys.argv[1:5]

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith("/cron"):
            with open(checkin_log, "a") as f:
                f.write(self.path + "\n")
            self.send_response(200)
            self.end_headers()
            return
        with open(status_file) as f:
            status = int(f.read().strip())
        with open(body_file, "rb") as f:
            body = f.read()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        pass

with socketserver.TCPServer(("127.0.0.1", int(port)), Handler) as httpd:
    httpd.serve_forever()
PYEOF

PORT=$((20000 + RANDOM % 20000))
python3 "$TMP/server.py" "$PORT" "$STATUS_FILE" "$BODY_FILE" "$CHECKIN_LOG" &
SERVER_PID=$!

for _ in $(seq 1 50); do
  curl -fsS -o /dev/null "http://127.0.0.1:$PORT/" 2>/dev/null && break
  sleep 0.1
done

HEALTH_URL="http://127.0.0.1:$PORT/api/health/deep"
SENTRY_CRON_URL="http://127.0.0.1:$PORT/cron"

fail=0
check() { # name expected_rc [extra_env...]
  local name="$1" want="$2" got
  shift 2
  # Truncate first, so checkin_sent below reads only THIS run's check-ins.
  : > "$CHECKIN_LOG"
  ( env HEALTH_URL="$HEALTH_URL" SENTRY_CRON_URL="$SENTRY_CRON_URL" "$@" "$GUARD" ) >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"
    fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

# Assert what actually REACHED the cron endpoint, not just the guard's exit code.
#
# The mock server captures every check-in to $CHECKIN_LOG precisely so the
# `status=` it sends can be verified, and nothing read it back. That mattered
# more than it sounds: if checkin() always posted status=ok, or dropped the
# status argument entirely, every exit-code assertion in this file would still
# pass - and the one mechanism in this change that pages a human would be dead
# while the suite stayed green.
checkin_sent() { # expected: ok | error | none
  local want="$1" got
  got=$(grep -oE 'status=(ok|error)' "$CHECKIN_LOG" 2>/dev/null | tail -1)
  got="${got#status=}"
  [ -z "$got" ] && got="none"
  if [ "$got" != "$want" ]; then
    echo "FAIL: check-in was status=$got, expected status=$want"
    fail=1
  else
    echo "   -> checked in status=$got"
  fi
}

echo 200 > "$STATUS_FILE"; healthy_body > "$BODY_FILE"
check "healthy body passes" 0
checkin_sent "ok"

check "missing SENTRY_CRON_URL is refused, not silently skipped" 1 env -u SENTRY_CRON_URL
checkin_sent "none"

echo 500 > "$STATUS_FILE"
check "non-200 is refused" 1
checkin_sent "error"
echo 200 > "$STATUS_FILE"

cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":false},
 "rpcs":{"1":{"status":"ok","last_block":100}}}
JSON
check "monitor.data_fresh false is refused" 1
checkin_sent "error"

cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"1":{"status":"error","last_block":100}}}
JSON
check "an unhealthy rpc is refused" 1

cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"error"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"1":{"status":"ok","last_block":100}}}
JSON
check "postgres not ok is refused" 1

cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"error"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"1":{"status":"ok","last_block":100}}}
JSON
check "redis not ok is refused" 1

# A stalled last_block only shows up after enough consecutive checks that
# report the same block - the failure the ticket calls out as distinct from
# rpcs.*.status, since the RPC connection itself can stay "ok" throughout.
healthy_body > "$BODY_FILE"
rm -f "$STATE_FILE"
check "1st check of a flat block passes (below threshold)" 0 env STALL_THRESHOLD=2 HEALTH_STATE_FILE="$STATE_FILE"
check "2nd check of a flat block passes (still below threshold)" 0 env STALL_THRESHOLD=2 HEALTH_STATE_FILE="$STATE_FILE"
check "3rd consecutive flat block trips the stall threshold" 1 env STALL_THRESHOLD=2 HEALTH_STATE_FILE="$STATE_FILE"

rm -f "$STATE_FILE"
cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"1":{"status":"ok","last_block":100}}}
JSON
check "1st check seeds state" 0 env STALL_THRESHOLD=2 HEALTH_STATE_FILE="$STATE_FILE"
cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"1":{"status":"ok","last_block":200}}}
JSON
check "an advancing block never trips the stall threshold" 0 env STALL_THRESHOLD=2 HEALTH_STATE_FILE="$STATE_FILE"

# The stall tracker's own error sentinel must fail closed like every other
# field here, not get treated as "nothing is stalled" - a state file the
# tracker cannot write to should refuse the run, not pass it quietly.
# Permission bits are ignored for root, so as root the nested mkdir would
# succeed, the state write would succeed, and this case would test nothing
# while appearing to fail. GitHub's ubuntu-latest is non-root; say so rather
# than leaving it an assumption baked into a mode string.
if [ "$(id -u)" -eq 0 ]; then
  echo "SKIP: fail-closed case needs a non-root user (permission bits do not apply to root)"
else
mkdir -m 500 "$TMP/readonly"
check "stall-check erroring (unwritable state dir) fails closed" 1 \
  env STALL_THRESHOLD=2 HEALTH_STATE_FILE="$TMP/readonly/nested/state.json"
chmod 700 "$TMP/readonly"
fi

[ "$fail" -eq 0 ] && echo "check-health-deep.sh behaves as documented"
exit "$fail"
