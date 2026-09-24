#!/usr/bin/env bash
# Proves check-file-size.sh goes red and green on the cases it claims to.
#
# Runs against a throwaway repo, not this one - the guard's own report count
# would otherwise rewrite itself every time someone added a line anywhere in
# the tree, which is exactly the kind of test CLAUDE.md warns is worse than no
# test at all.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-file-size.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
check() { # name expected_rc
  local name="$1" want="$2" got
  ( cd "$TMP" && LINE_LIMIT=5 BASE_REF=base "$GUARD" ) >/tmp/check-file-size.test.out 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"
    cat /tmp/check-file-size.test.out
    fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

lines() { # n
  for _ in $(seq "$1"); do echo line; done
}

cd "$TMP"
git init -q .
git config user.email t@t; git config user.name t

# A fixture with a known shape: one file under the limit, one already over it.
# The report has to name exactly these two, not "whatever is in the repo".
lines 3 > short.rs
lines 8 > already_over.rs
git add -A && git commit -qm "base commit"
git branch base

report="$(cd "$TMP" && LINE_LIMIT=5 BASE_REF=base "$GUARD")"
if printf '%s\n' "$report" | grep -q "files over 5 lines: 1 (8 lines total)" \
   && printf '%s\n' "$report" | grep -q "already_over.rs"; then
  echo "ok: report names the one file over the limit with the right total"
else
  echo "FAIL: report did not match the known fixture:"
  printf '%s\n' "$report"
  fail=1
fi
check "an untouched change (nothing over the limit grew) passes" 0

lines 4 > short.rs
git add -A && git commit -qm "still under the limit"
check "editing a file that stays under the limit passes" 0

lines 8 >> short.rs
git add -A && git commit -qm "crosses the limit for the first time"
check "a file crossing the limit for the first time is refused" 1

git reset -q --hard HEAD~1
check "green again once that commit is gone" 0

lines 6 >> already_over.rs
git add -A && git commit -qm "grows further while already over"
check "a file already over the limit that grows further is refused" 1

git reset -q --hard HEAD~1
check "green again once that commit is gone too" 0

lines 7 > already_over.rs
git add -A && git commit -qm "shrinks but is still over the limit"
check "a file shrinking while still over the limit passes" 0

lines 3 > already_over.rs
git add -A && git commit -qm "shrinks back under the limit entirely"
check "a file shrinking back under the limit passes" 0

lines 9 > brand_new.rs
git add -A && git commit -qm "a brand new file starts over the limit"
check "a brand new file over the limit is refused" 1

git reset -q --hard HEAD~1
check "green again once the new file is gone" 0

# A BASE_REF that doesn't resolve at all (e.g. a fetch step upstream failed)
# must not be treated as "nothing grew" - it narrows to HEAD~1 instead and
# still has to catch real growth there, or the fallback would just be a
# quieter version of the same silent-pass bug.
lines 8 > short.rs
git add -A && git commit -qm "grows past the limit, to be seen via the HEAD~1 fallback"
out="$(cd "$TMP" && LINE_LIMIT=5 BASE_REF=does-not-exist "$GUARD" 2>&1)"
rc=$?
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::warning::does-not-exist not available; checking the previous commit only'; then
  echo "ok: an unresolvable BASE_REF falls back to HEAD~1 and still catches real growth"
else
  echo "FAIL: an unresolvable BASE_REF should fall back and still enforce, not silently pass"
  printf '%s\n' "$out"
  fail=1
fi
git reset -q --hard HEAD~1

# A BASE_REF that exists but shares no history with HEAD (e.g. a shallow
# checkout that never fetched a common ancestor) makes the triple-dot diff
# itself fail, not just return empty. That must surface as a visible warning
# and retry against HEAD~1 - a real, narrower check - not a silent skip.
main_branch="$(git branch --show-current)"
git checkout -q --orphan disjoint
git commit -q --allow-empty -m "unrelated root, no shared history with main"
git checkout -q "$main_branch"
git branch -f disjoint_base disjoint
out="$(cd "$TMP" && LINE_LIMIT=5 BASE_REF=disjoint_base "$GUARD" 2>&1)"
rc=$?
if [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q '::warning::git diff against disjoint_base failed'; then
  echo "ok: a BASE_REF with no shared history warns and retries against HEAD~1 instead of silently passing"
else
  echo "FAIL: a failed base-ref diff should warn and retry, not silently claim clean"
  printf '%s\n' "$out"
  fail=1
fi
git branch -D disjoint disjoint_base >/dev/null

# When there is truly nothing to compare against - no BASE_REF and no
# HEAD~1 either, e.g. a repo's first commit - the guard has no way to know
# whether anything grew. Passing here would be the exact bug this script
# exists to catch; it fails the build instead.
FIRST="$(mktemp -d)"
( cd "$FIRST" && git init -q . && git config user.email t@t && git config user.name t \
  && lines 8 > only.rs && git add -A && git commit -qm "first commit, nothing to diff against" )
out="$(cd "$FIRST" && LINE_LIMIT=5 BASE_REF=does-not-exist "$GUARD" 2>&1)"
rc=$?
rm -rf "$FIRST"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::error::no base commit to diff against'; then
  echo "ok: no base at all fails the build instead of passing it"
else
  echo "FAIL: a guard with nothing to diff against should fail closed, not pass"
  printf '%s\n' "$out"
  fail=1
fi

[ "$fail" -eq 0 ] && echo "check-file-size.sh behaves as documented"
exit "$fail"
