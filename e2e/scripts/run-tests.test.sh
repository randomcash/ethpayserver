#!/usr/bin/env bash
# Proves run-tests.sh's exit trap actually removes its scratch TMPDIR when
# the run is interrupted - not a comment asserting someone Ctrl-C'd it once
# and watched by hand.
#
# A fake `npx` stands in for the real one: it records the TMPDIR it was
# handed, then blocks so the harness can interrupt it like a real Ctrl-C
# would. If the trap didn't fire on a signal - only on a clean exit - this
# would catch it.
#
# `set -m` and signaling the process group (`-"$pid"`), not just the wrapper's
# own pid, both matter: bash ignores SIGINT for an async (`&`) job in a
# non-interactive script unless job control is on, and a real terminal Ctrl-C
# hits the whole foreground process group at once, not the parent alone. Get
# either wrong and `kill -INT` is a no-op - the fake npx runs its full 60s
# sleep and the script exits cleanly regardless of whether the trap works,
# which passed here once before catching nothing.
set -uo pipefail
set -m

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="$E2E_DIR/scripts/run-tests.sh"
FAKE_BIN="$(mktemp -d)"
MARKER="$(mktemp)"
trap 'rm -rf "$FAKE_BIN" "$MARKER" "$E2E_DIR/.tmp"' EXIT

fail=0

cat > "$FAKE_BIN/npx" <<EOF
#!/usr/bin/env bash
echo "\$TMPDIR" > "$MARKER"
sleep 60
EOF
chmod +x "$FAKE_BIN/npx"

cd "$E2E_DIR"
rm -rf .tmp

PATH="$FAKE_BIN:$PATH" "$RUNNER" &
pid=$!

# Wait for the fake npx to report the scratch dir run-tests.sh handed it.
for _ in $(seq 1 50); do
  [ -s "$MARKER" ] && break
  sleep 0.1
done
scratch="$(cat "$MARKER")"

if [ -n "$scratch" ] && [ -d "$scratch" ]; then
  echo "ok: scratch dir exists while the run is in flight ($scratch)"
else
  echo "FAIL: scratch dir was never created before interrupting"
  fail=1
fi

kill -INT -"$pid" 2>/dev/null
wait "$pid" 2>/dev/null

if [ -n "$scratch" ] && [ ! -d "$scratch" ]; then
  echo "ok: scratch dir removed after SIGINT"
else
  echo "FAIL: scratch dir survived SIGINT - $scratch"
  fail=1
fi

[ "$fail" -eq 0 ] && echo "run-tests.sh behaves as documented"
exit "$fail"
