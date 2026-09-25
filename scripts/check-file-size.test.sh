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
# A broken listing (e.g. an unreadable index) is not "nothing is over the
# limit" - it must fail the build rather than report a false-clean zero, the
# same conflation the ratchet below is built to refuse.
#
# A directory in place of the index file, not chmod 000: permission bits are
# bypassed when the runner is root (common on containerized/self-hosted CI),
# which would make this fault injection a no-op there and turn a real failure
# into a spurious pass. `git ls-files` can't read a directory as an index file
# regardless of who is asking.
mv "$TMP/.git/index" "$TMP/.git/index.bak"
mkdir "$TMP/.git/index"
out="$(cd "$TMP" && LINE_LIMIT=5 BASE_REF=base "$GUARD" 2>&1)"
rc=$?
rmdir "$TMP/.git/index"
mv "$TMP/.git/index.bak" "$TMP/.git/index"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::error::git ls-files failed'; then
  echo "ok: a broken git ls-files fails the build instead of reporting a false-clean zero"
else
  echo "FAIL: a broken git ls-files should fail closed, not report zero files over the limit"
  printf '%s\n' "$out"
  fail=1
fi

# A path git tracks but that isn't a regular file in the working tree (e.g. a
# bad checkout leaving a directory where a file should be) breaks `wc -l` the
# same way a broken `git ls-files` breaks the listing above - and must fail
# the same way, not silently read as 0 lines and vanish from the report.
#
# A directory in place of the file, not chmod: same reasoning as the
# git-ls-files fault injection above - permission bits are a no-op against
# root, but you can never `wc -l` a directory's contents regardless of who's
# asking.
rm short.rs
mkdir short.rs
out="$(cd "$TMP" && LINE_LIMIT=5 BASE_REF=base "$GUARD" 2>&1)"
rc=$?
rmdir short.rs
lines 3 > short.rs
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::error::wc -l failed for short.rs'; then
  echo "ok: a tracked path that isn't a regular file fails the report instead of reading as 0 lines"
else
  echo "FAIL: an unreadable tracked file should fail the report, not silently report 0 lines"
  printf '%s\n' "$out"
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

# The disjoint-history retry above only proved it can pass (nothing had grown
# at that point) - it has to actually catch growth too, or "warns and
# retries" could retry into a check that never fires.
lines 8 > short.rs
git add -A && git commit -qm "grows past the limit again, to be seen via the disjoint-diff retry"
git checkout -q --orphan disjoint2
git commit -q --allow-empty -m "another unrelated root, no shared history with main"
git checkout -q "$main_branch"
git branch -f disjoint2_base disjoint2
out="$(cd "$TMP" && LINE_LIMIT=5 BASE_REF=disjoint2_base "$GUARD" 2>&1)"
rc=$?
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::warning::git diff against disjoint2_base failed' \
   && printf '%s\n' "$out" | grep -q 'short.rs grew from 4 to 8'; then
  echo "ok: the disjoint-history retry against HEAD~1 still catches real growth, not just a pass"
else
  echo "FAIL: the disjoint-history retry should still enforce growth, not just prove it can pass"
  printf '%s\n' "$out"
  fail=1
fi
git reset -q --hard HEAD~1
git branch -D disjoint2 disjoint2_base >/dev/null

# A failed diff with no HEAD~1 to retry against either (a repo's first commit,
# but this time BASE_REF resolves to something with no shared history rather
# than failing to resolve at all) has no fallback left - it must fail closed,
# not skip.
NOHEAD1="$(mktemp -d)"
(
  cd "$NOHEAD1" && git init -q . && git config user.email t@t && git config user.name t
  lines 8 > only.rs && git add -A && git commit -qm "first commit, nothing to diff against"
  first_branch="$(git branch --show-current)"
  git checkout -q --orphan unrelated
  git commit -q --allow-empty -m "unrelated root, no shared history"
  git branch -f unrelated_base unrelated
  git checkout -q "$first_branch"
)
out="$(cd "$NOHEAD1" && LINE_LIMIT=5 BASE_REF=unrelated_base "$GUARD" 2>&1)"
rc=$?
rm -rf "$NOHEAD1"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::error::git diff against unrelated_base failed'; then
  echo "ok: a failed diff with no HEAD~1 to retry against fails the build"
else
  echo "FAIL: a failed diff with no HEAD~1 should fail closed, not skip"
  printf '%s\n' "$out"
  fail=1
fi

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

# A pure rename of an already-oversized file has no content at its new path
# at base, so a naive lookup there fails and used to be read as "the file is
# new" (before=0) - reporting an unchanged file as having grown from 0 lines
# and refusing it for the exact split work this guard is meant to protect.
RENAME="$(mktemp -d)"
(
  cd "$RENAME" && git init -q . && git config user.email t@t && git config user.name t
  lines 8 > old_name.rs
  git add -A && git commit -qm "base commit, already over the limit"
  git branch base
  git mv old_name.rs new_name.rs
  git commit -qm "pure rename, no content change"
)
out="$(cd "$RENAME" && LINE_LIMIT=5 BASE_REF=base "$GUARD" 2>&1)"
rc=$?
rm -rf "$RENAME"
if [ "$rc" -eq 0 ] && ! printf '%s\n' "$out" | grep -q 'grew from'; then
  echo "ok: renaming an already-oversized file with no content change passes"
else
  echo "FAIL: a pure rename of an oversized file must not read as growing from 0"
  printf '%s\n' "$out"
  fail=1
fi

# The rename fix must still catch real growth - looked up against the file's
# actual size at its old path, not against a 0 that would mask growth that
# should be refused.
RENAME2="$(mktemp -d)"
(
  cd "$RENAME2" && git init -q . && git config user.email t@t && git config user.name t
  lines 8 > old_name.rs
  git add -A && git commit -qm "base commit, already over the limit"
  git branch base
  git mv old_name.rs new_name.rs
  lines 4 >> new_name.rs
  git commit -qm "rename and grow further while already over"
)
out="$(cd "$RENAME2" && LINE_LIMIT=5 BASE_REF=base "$GUARD" 2>&1)"
rc=$?
rm -rf "$RENAME2"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'new_name.rs grew from 8 to 12'; then
  echo "ok: a rename that also grows the file further is still refused, against its real size at base"
else
  echo "FAIL: a rename that grows further should be refused against its real prior size, not 0"
  printf '%s\n' "$out"
  fail=1
fi

# An unreadable base blob for a modified (not renamed, not new) file must
# fail the build with a clear error, not silently read as before=0 - the same
# conflation already fixed above for renames, but here triggered by a missing
# git object rather than a path that never existed at base. A file that
# shrinks while staying over the limit would otherwise misread as "grew from
# 0" and fail for the wrong reason instead of passing.
GITSHOW="$(mktemp -d)"
(
  cd "$GITSHOW" && git init -q . && git config user.email t@t && git config user.name t
  lines 8 > already_over.rs
  git add -A && git commit -qm "base commit, already over the limit"
  git branch base
  lines 6 > already_over.rs
  git commit -qam "shrinks while still over the limit"
)
base_blob="$(cd "$GITSHOW" && git rev-parse base:already_over.rs)"
obj_path="$GITSHOW/.git/objects/${base_blob:0:2}/${base_blob:2}"
mv "$obj_path" "$obj_path.bak"
out="$(cd "$GITSHOW" && LINE_LIMIT=5 BASE_REF=base "$GUARD" 2>&1)"
rc=$?
mv "$obj_path.bak" "$obj_path"
rm -rf "$GITSHOW"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q '::error::git show base:already_over.rs failed'; then
  echo "ok: an unreadable base blob fails the build instead of misreading it as a new file"
else
  echo "FAIL: a failed git show for a modified file should fail closed, not read as before=0"
  printf '%s\n' "$out"
  fail=1
fi

[ "$fail" -eq 0 ] && echo "check-file-size.sh behaves as documented"
exit "$fail"
