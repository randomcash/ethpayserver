#!/usr/bin/env bash
# Guards the wasmtime advisories ignored in .cargo/audit.toml.
#
# Those ignores are valid for two reasons, and each is a property of the build
# today that nothing else would notice changing:
#
#   1. wasmtime is compiled without its GC heap and component model. A new
#      dependency, a workspace change or an --all-features run can switch either
#      on through feature unification; the ignore would then cover a real
#      exposure while the comment beside it still read as true. So this fails if
#      any of those features is enabled.
#   2. They are a stopgap for a version that is not fixed yet. Once the locked
#      wasmtime is a fixed release (>=48.0.4, or >=49.0.2) the advisories no
#      longer apply and the ignores are dead weight: an allowlist with no end is
#      how a stopgap becomes permanent. So this fails if the lock
#      has reached a fixed release while any of them remains.
#
# Test hooks (used only by check-wasmtime-ignores.test.sh): WASMTIME_FEATURES_FILE
# replaces the `cargo tree` output, LOCK_FILE and AUDIT_FILE replace the paths.
set -uo pipefail

LOCK="${LOCK_FILE:-Cargo.lock}"
AUDIT="${AUDIT_FILE:-.cargo/audit.toml}"
IDS="RUSTSEC-2026-0315 RUSTSEC-2026-0316 RUSTSEC-2026-0325 RUSTSEC-2026-0326 RUSTSEC-2026-0327"
BANNED='gc|gc-drc|gc-null|gc-copying|component-model|component-model-async|async'

if [ -n "${WASMTIME_FEATURES_FILE:-}" ]; then
  tree="$(cat "$WASMTIME_FEATURES_FILE")"
else
  tree="$(cargo tree --locked -e features -i wasmtime 2>&1)" || { echo "cargo tree failed:"; echo "$tree"; exit 1; }
fi

rc=0

on="$(grep -oE "wasmtime feature \"($BANNED)\"" <<<"$tree" | sort -u)"
if [ -n "$on" ]; then
  echo "FAIL: wasmtime has a feature enabled that the audit ignores assume is off:"
  echo "$on" | sed 's/^/  /'
  echo "The advisories ignored in $AUDIT need the GC heap or the component model."
  echo "Either stop enabling the feature or remove the ignores and take the wasmtime fix."
  rc=1
fi

ver="$(awk '/^name = "wasmtime"$/ {getline; gsub(/[^0-9.]/, "", $0); print; exit}' "$LOCK")"
if [ -z "$ver" ]; then
  echo "FAIL: wasmtime not found in $LOCK"
  rc=1
else
  ge() { [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -1)" = "$2" ]; }
  fixed=0
  if ge "$ver" 48.0.4 && ! { ge "$ver" 49.0.0 && ! ge "$ver" 49.0.2; }; then fixed=1; fi
  if [ "$fixed" = 1 ]; then
    left="$(for id in $IDS; do grep -q "\"$id\"" "$AUDIT" && printf '%s ' "$id"; done)"
    if [ -n "$left" ]; then
      echo "FAIL: wasmtime $ver is a fixed release but $AUDIT still ignores: $left"
      echo "Remove those ignores; they only hide what the fix already closed."
      rc=1
    fi
  fi
fi

[ "$rc" = 0 ] && echo "ok: wasmtime ${ver:-?} builds without gc/component-model/async and the ignores are still needed"
exit "$rc"
