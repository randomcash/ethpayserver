#!/usr/bin/env bash
# Proves check-sentry-release.sh actually refuses an empty SENTRY_RELEASE
# rather than compiling clean and looking identical to success - the guard's
# whole job is to catch that, and in normal CI operation github.sha is always
# non-empty, so nothing organically exercises the failing branch. Without this,
# a rename or reordering that broke the guard itself would go unnoticed.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$HERE/check-sentry-release.sh"

fail=0
check() {  # name expected_rc value
  local name="$1" want="$2" value="$3" got
  "$CHECK" "$value" >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"; fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

check "a normal 7-char sha passes" 0 "abc1234"
check "an empty value is refused" 1 ""

if [ "$fail" -ne 0 ]; then
  echo "check-sentry-release.sh does NOT behave as documented"; exit 1
fi
echo "check-sentry-release.sh behaves as documented"
