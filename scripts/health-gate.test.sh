#!/usr/bin/env bash
# Exercises scripts/health-gate.sh against a real HTTP server.
#
# This gate decides whether a deploy is allowed to stand. Until now it had no
# test at all, while CI runs it post-deploy - so every one of its refusal
# branches was code nobody had ever seen refuse anything.
#
# The case that motivated the newest branch is the one worth understanding: an
# EMPTY `rpcs` map means evmmonitor is unreachable or the chain-health fetch is
# itself erroring. There is then no chain to name as bad, so the "all chains ok"
# check passes VACUOUSLY - it is satisfied by there being nothing to check. The
# gate would let a cutover through with the monitor reporting nothing at all.
# `monitor.data_fresh` is the independent signal that catches it.
#
# HEALTH_TIMEOUT is kept at 1s throughout: every refusal path sleeps and retries
# until the timeout, so a generous timeout here buys nothing but a slow suite.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="$HERE/health-gate.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"; kill "${SRV:-}" 2>/dev/null' EXIT

BODY_FILE="$TMP/body.json"
HEADER_FILE="$TMP/sentry_release"
PORT=$((9000 + RANDOM % 900))

cat > "$TMP/server.py" <<'PY'
import http.server, sys
body_file, header_file = sys.argv[2], sys.argv[3]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(s):
        body = open(body_file, "rb").read()
        release = open(header_file).read().strip()
        s.send_response(200)
        s.send_header("Content-Type", "application/json")
        s.send_header("Content-Length", str(len(body)))
        s.send_header("X-Sentry-Release", release)
        s.end_headers()
        s.wfile.write(body)
    def log_message(s, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PY

python3 "$TMP/server.py" "$PORT" "$BODY_FILE" "$HEADER_FILE" &
SRV=$!
for _ in $(seq 1 50); do
  echo '{}' > "$BODY_FILE"
  echo 'abc1234' > "$HEADER_FILE"
  curl -fsS -o /dev/null "http://127.0.0.1:$PORT/" 2>/dev/null && break
  sleep 0.1
done

fail=0
check() {  # name expected_rc
  local name="$1" want="$2" got
  ( env HEALTH_URL="http://127.0.0.1:$PORT/health/deep" HEALTH_TIMEOUT=1 bash "$GATE" ) >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"; fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

healthy() {
  cat > "$BODY_FILE" <<'JSON'
{"build_sha":"abc1234","postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"eip155:11155111":{"status":"ok","last_block":100}}}
JSON
}

healthy
check "a healthy response passes" 0

# The branch this suite exists for.
cat > "$BODY_FILE" <<'JSON'
{"build_sha":"abc1234","postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":false},
 "rpcs":{"eip155:11155111":{"status":"ok","last_block":100}}}
JSON
check "data_fresh false is refused" 1

# The vacuous pass: nothing is wrong because nothing is reported.
cat > "$BODY_FILE" <<'JSON'
{"build_sha":"abc1234","postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":false},
 "rpcs":{}}
JSON
check "an empty rpcs map with data_fresh false is refused, not passed vacuously" 1

# build_sha and the Sentry release are set by two separate CI steps from the
# same commit sha, so a rename, typo, or a rebuild stage that drops
# SENTRY_RELEASE lets them drift apart silently - a healthy body with a
# build_sha that matches EXPECTED_SHA is not enough on its own.
healthy
echo '9999999' > "$HEADER_FILE"
check "a build_sha/sentry-release mismatch is refused" 1

echo '' > "$HEADER_FILE"
check "a missing x-sentry-release header is refused" 1

# The vacuous pass this comparison could produce: a malformed response with no
# build_sha at all, paired with a missing header, satisfies "" == "" unless
# the empty header is rejected outright regardless of what build_sha says.
cat > "$BODY_FILE" <<'JSON'
{"postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"eip155:11155111":{"status":"ok","last_block":100}}}
JSON
echo '' > "$HEADER_FILE"
check "a missing build_sha and a missing header is refused, not passed vacuously" 1

# Restore before the remaining cases, which are not testing this header.
healthy
echo 'abc1234' > "$HEADER_FILE"

# ... and the same shape with a missing monitor key entirely, which is what an
# older server or a partial response looks like. The extractor defaults to
# False, so this must refuse rather than read absence as health.
cat > "$BODY_FILE" <<'JSON'
{"build_sha":"abc1234","postgres":{"status":"ok"},"redis":{"status":"ok"},
 "rpcs":{}}
JSON
check "a missing monitor key is refused" 1

cat > "$BODY_FILE" <<'JSON'
{"build_sha":"abc1234","postgres":{"status":"error"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"eip155:11155111":{"status":"ok","last_block":100}}}
JSON
check "postgres not ok is refused" 1

cat > "$BODY_FILE" <<'JSON'
{"build_sha":"abc1234","postgres":{"status":"ok"},"redis":{"status":"ok"},
 "monitor":{"status":"ok","data_fresh":true},
 "rpcs":{"eip155:11155111":{"status":"error","last_block":100}}}
JSON
check "an unhealthy chain is refused" 1

if [ "$fail" -ne 0 ]; then
  echo "health-gate.sh does NOT behave as documented"; exit 1
fi
echo "health-gate.sh behaves as documented"
