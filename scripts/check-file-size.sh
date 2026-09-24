#!/usr/bin/env bash
# Reports every .rs file over the line limit, and fails the build on the ones
# that got worse in this change.
#
# An architecture audit found 26 files over 400 lines on 2026-09-14 and 47 of
# them nine days later - nearly double, on both file count and total lines -
# despite a whole series of split tickets landing in that window. The
# splitting happened; the growth outpaced it. The reason is that nothing ever
# checked: the limit lived in an audit issue that someone had to remember to
# rerun by hand. A convention nothing enforces is a preference, and the growth
# curve is what a preference looks like.
#
# Failing on all 47 existing files the day this lands is how a gate gets
# switched off in its first week - see CLAUDE.md's account of exactly that
# happening to other checks. So the backlog is left alone: what fails is a
# file getting WORSE in this change, either by crossing the limit for the
# first time or by growing further while already over it. A file that shrinks
# - even while still over the limit - always passes. That is a direct answer
# to "doubled in nine days": the doubling was entirely new and modified code,
# and this refuses exactly that pattern from here on without demanding the
# backlog be fixed first.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

LINE_LIMIT="${LINE_LIMIT:-400}"
BASE_REF="${BASE_REF:-origin/testnet}"
status=0

# Report: every .rs file over the limit right now, worst first, independent of
# the ratchet below. This is what a human audit reads instead of hand-running
# `wc -l` over the tree again.
#
# A failed listing is not the same thing as "nothing is over the limit" - the
# exact conflation this script exists to catch - so it fails the build rather
# than reporting a clean zero.
if ! files="$(git ls-files '*.rs')"; then
  echo "::error::git ls-files failed - cannot measure current file sizes" >&2
  exit 1
fi

report="$(printf '%s\n' "$files" | while read -r f; do
  [ -z "$f" ] && continue
  n="$(wc -l < "$f")"
  if [ "$n" -gt "$LINE_LIMIT" ]; then
    printf '%d %s\n' "$n" "$f"
  fi
done | sort -rn)"

count=0
total=0
if [ -n "$report" ]; then
  count="$(printf '%s\n' "$report" | wc -l)"
  total="$(printf '%s\n' "$report" | awk '{s+=$1} END{print s+0}')"
fi

echo "files over $LINE_LIMIT lines: $count ($total lines total)"
if [ -n "$report" ]; then
  printf '%s\n' "$report" | while read -r n f; do
    printf '    %6d  %s\n' "$n" "$f"
  done
fi

# Ratchet: only files this change actually touches can have "grown in this
# change", so a file the diff never mentions is never a candidate here no
# matter how far over the limit it already sits.
#
# A base that can't be resolved or diffed is not the same thing as "nothing
# grew" - that conflation is the exact bug this script exists to catch, so it
# does not skip on either failure. It narrows to HEAD~1 instead (same move
# check-no-session-urls.sh makes for the same reason), and only gives up - by
# failing the build, not passing it - once there is truly nothing left to
# compare against.
base="$BASE_REF"
if ! git rev-parse --verify --quiet "$base" >/dev/null 2>&1; then
  echo "::warning::$BASE_REF not available; checking the previous commit only" >&2
  base="HEAD~1"
fi

if ! git rev-parse --verify --quiet "$base" >/dev/null 2>&1; then
  echo "::error::no base commit to diff against ($BASE_REF and HEAD~1 both unavailable) - failing rather than skipping the growth check" >&2
  exit 1
fi

if ! changed="$(git diff --name-status -M --diff-filter=ACMR "${base}...HEAD" -- '*.rs' 2>&1)"; then
  if [ "$base" != "HEAD~1" ] && git rev-parse --verify --quiet "HEAD~1" >/dev/null 2>&1; then
    echo "::warning::git diff against $base failed ($changed); retrying against the previous commit only" >&2
    base="HEAD~1"
    changed="$(git diff --name-status -M --diff-filter=ACMR "${base}...HEAD" -- '*.rs' 2>&1)" || {
      echo "::error::git diff against $base failed too ($changed) - failing rather than skipping the growth check" >&2
      exit 1
    }
  else
    echo "::error::git diff against $base failed ($changed) - failing rather than skipping the growth check" >&2
    exit 1
  fi
fi
# --name-status (not --name-only) so a rename or copy carries its source path
# alongside its destination. --diff-filter=ACMR includes renames, and
# --name-only alone would give only the new path - so "before" was being
# looked up at a path that never existed there, git show failed, and that
# failure was read as "the file is new" (before=0). Renaming an
# already-oversized file - exactly the "split, worst first" work this script
# exists to make safe - would then read as growing from 0 lines and fail the
# build for a file that never changed.
while IFS=$'\t' read -r dstatus path1 path2; do
  [ -z "$dstatus" ] && continue
  case "$dstatus" in
    R*|C*) old="$path1"; f="$path2" ;;
    *) old="$path1"; f="$path1" ;;
  esac
  [ -f "$f" ] || continue
  after="$(wc -l < "$f")"
  case "$dstatus" in
    A*)
      # A genuinely new path has nothing to look up at base - 0 is the
      # correct answer here, not a swallowed failure standing in for one.
      before=0
      ;;
    *)
      before="$(git show "${base}:${old}" 2>/dev/null | wc -l)"
      if [ "${PIPESTATUS[0]}" -ne 0 ]; then
        echo "::error::git show ${base}:${old} failed - cannot determine $f's size before this change" >&2
        exit 1
      fi
      ;;
  esac
  over_after=$(( after > LINE_LIMIT ? after - LINE_LIMIT : 0 ))
  over_before=$(( before > LINE_LIMIT ? before - LINE_LIMIT : 0 ))
  if [ "$over_after" -gt "$over_before" ]; then
    status=1
    echo "::error::$f grew from $before to $after lines, past the ${LINE_LIMIT}-line limit ($over_before -> $over_after lines over)" >&2
  fi
done <<< "$changed"

if [ "$status" -eq 0 ]; then
  echo "no .rs file grew past the ${LINE_LIMIT}-line limit in this change"
else
  echo "  Split the file rather than growing it further. Along the behaviours" >&2
  echo "  it covers, not by arbitrary halves - see CLAUDE.md." >&2
fi

exit "$status"
