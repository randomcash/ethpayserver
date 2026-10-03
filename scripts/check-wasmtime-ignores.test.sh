#!/usr/bin/env bash
# Proves check-wasmtime-ignores.sh goes red on the cases it claims to, and stays
# green on the one it must not trip. A guard that only ever passed proves nothing.
set -uo pipefail
GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-wasmtime-ignores.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
fail=0

lock() { printf '[[package]]\nname = "wasmtime"\nversion = "%s"\n' "$1" > "$TMP/Cargo.lock"; }
audit() { printf '[advisories]\nignore = [\n%s]\n' "$1" > "$TMP/audit.toml"; }
tree() { printf 'wasmtime v47.0.4\n├── wasmtime feature "cranelift"\n├── wasmtime feature "runtime"\n%s' "$1" > "$TMP/tree"; }
ALL='    "RUSTSEC-2026-0315",
    "RUSTSEC-2026-0316",
    "RUSTSEC-2026-0325",
    "RUSTSEC-2026-0326",
    "RUSTSEC-2026-0327",
'
check() { # name want-rc
  WASMTIME_FEATURES_FILE="$TMP/tree" LOCK_FILE="$TMP/Cargo.lock" AUDIT_FILE="$TMP/audit.toml" "$GUARD" >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$2" ]; then echo "FAIL: $1 - expected exit $2, got $got"; fail=1; else echo "ok: $1 (exit $got)"; fi
}

lock 47.0.4; audit "$ALL"; tree ''
check "the build as it is today passes" 0
tree '├── wasmtime feature "gc"
'
check "gc enabled is refused" 1
tree '├── wasmtime feature "component-model"
'
check "component-model enabled is refused" 1
tree '├── wasmtime feature "async"
'
check "async enabled is refused" 1
tree '├── wasmtime feature "gc-null-extra"
'
check "an unrelated feature whose name merely starts with a banned one passes" 0
tree ''
lock 48.0.3; check "an affected release with the ignores passes" 0
lock 48.0.4; check "a fixed release that still ignores the advisories is refused" 1
lock 49.0.1; check "49.0.1 is still affected, so the ignores stand" 0
lock 49.0.2; check "49.0.2 is fixed and the ignores must go" 1
audit ''; check "fixed release with the ignores removed passes" 0
lock 47.0.4; audit ''; check "an affected release with no ignores passes (the guard does not require them)" 0
exit $fail
