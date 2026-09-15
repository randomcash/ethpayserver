#!/usr/bin/env bash
# Refuses a session URL in a commit message or a tracked file.
#
# This repository is public. CLAUDE.md has said "no session URLs in commits or
# PR bodies" since long before this script existed, and 58 commits between
# 2026-06-28 and 2026-09-15 carry one anyway - because the rule was written
# down and nothing ever checked it. A session link is a pointer into a
# transcript that can hold anything the session was shown, credentials
# included, and it is not something an outside reader of a public repository
# should be handed.
#
# Commit messages, not just files: that is where they actually land, and it is
# the surface no file-scanning guard would ever have caught.
#
# Scans the range BASE_REF..HEAD, so it judges what a change introduces rather
# than what history already contains - rewriting 647 commits of a branch whose
# images are tagged by commit sha is a bigger problem than the one it solves.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

BASE_REF="${BASE_REF:-origin/testnet}"
PATTERN='claude\.ai/code/session'
status=0

if git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
  range="${BASE_REF}..HEAD"
else
  # A shallow or detached checkout without the base: fall back to the single
  # commit at HEAD rather than silently scanning nothing.
  echo "::warning::$BASE_REF not available; checking HEAD only"
  range="HEAD~1..HEAD"
fi

offenders="$(git log --format='%H %s' "$range" 2>/dev/null | while read -r sha subject; do
  if git log -1 --format='%B' "$sha" | grep -qE "$PATTERN"; then
    printf '    %s %s\n' "${sha:0:8}" "$subject"
  fi
done)"

if [ -n "$offenders" ]; then
  status=1
  echo "::error::session URL in a commit message:" >&2
  printf '%s\n' "$offenders" >&2
  echo >&2
  echo "  Drop the 'Claude-Session:' trailer. Co-Authored-By is fine; the link" >&2
  echo "  is not - this repository is public and a transcript can hold secrets." >&2
fi

# git grep, not bare grep: grep here is ugrep and honours .gitignore, which has
# already produced a false-clean leak scan once.
# The guard's own test fixtures have to contain what it detects, so they are
# excluded by path rather than by a cleverer pattern that would be one escape
# away from matching nothing at all.
if files="$(git grep -nIE "$PATTERN" -- . ':!scripts/check-no-session-urls.test.sh' 2>/dev/null)"; then
  status=1
  echo "::error::session URL in a tracked file:" >&2
  printf '%s\n' "$files" >&2
fi

[ "$status" -eq 0 ] && echo "no session URLs in commits or tracked files"
exit "$status"
