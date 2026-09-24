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
report="$(git ls-files '*.rs' | while read -r f; do
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
if ! git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
  echo "::warning::$BASE_REF not available; skipping the growth check" >&2
  exit "$status"
fi

if ! changed="$(git diff --name-only --diff-filter=ACMR "${BASE_REF}...HEAD" -- '*.rs' 2>&1)"; then
  echo "::warning::git diff against $BASE_REF failed; skipping the growth check ($changed)" >&2
  exit "$status"
fi
while IFS= read -r f; do
  [ -z "$f" ] && continue
  [ -f "$f" ] || continue
  after="$(wc -l < "$f")"
  before="$(git show "${BASE_REF}:${f}" 2>/dev/null | wc -l)"
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
