#!/usr/bin/env bash
# Proves check-no-session-urls.sh goes red on the cases it claims to.
#
# The guard exists because a rule that nothing checks is not a rule, so
# "it passed on a clean tree" is not evidence that it works.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-no-session-urls.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
check() { # name expected_rc
  local name="$1" want="$2" got
  ( cd "$TMP" && BASE_REF=base "$GUARD" ) >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"
    fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

cd "$TMP"
git init -q .
git config user.email t@t; git config user.name t
echo "code" > src.rs
git add -A && git commit -qm "base commit"
git branch base

echo "more" >> src.rs
git add -A && git commit -qm "an ordinary commit

Co-Authored-By: Someone <x@y>"
check "a clean commit passes" 0

git commit -q --allow-empty -m "a commit with a session trailer

Co-Authored-By: Someone <x@y>
Claude-Session: https://claude.ai/code/session_01ABCDEF"
check "a session URL in a commit message is refused" 1

git reset -q --hard HEAD~1
check "green again once the commit is gone" 0

echo "// see https://claude.ai/code/session_01ABCDEF" >> src.rs
git add -A && git commit -qm "a session URL in a file"
check "a session URL in a tracked file is refused" 1

[ "$fail" -eq 0 ] && echo "check-no-session-urls.sh behaves as documented"
exit "$fail"
