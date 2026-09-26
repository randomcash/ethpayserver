#!/usr/bin/env bash
# This repository is public. Billing and subscription semantics are not.
#
# On 2026-09-26 a PR reached the public repo titled "Billing plugin: the
# instance sells subscriptions to itself", carrying the term in its branch
# name and its diff. It was closed, but a public PR's title, branch and diff
# cannot be retracted - and the sweep that followed found the term already
# merged 190 times, including in a migration FILENAME. The rule existed and
# nothing enforced it, which is the difference between a rule and a
# preference: `check-no-ticket-refs.sh` exists for the same reason.
#
# WHY A BASELINE RATHER THAN A CLEAN FAIL. Failing on all 190 today would
# block every open PR and get this switched off in its first week - exactly
# what check-file-size.sh's header warns about, and what we watched happen to
# a file-size gate this same morning. So the backlog is recorded per file and
# what fails is an INCREASE: a new file using the term, or an existing one
# using it more. The scrub that removes the backlog is tracked separately, and
# every entry here is a debt rather than a permission.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1
BASELINE="${COMMERCIAL_BASELINE:-scripts/commercial-terms-baseline.txt}"

# `billing` and `stripe` only. NOT `subscription`: this codebase subscribes to
# Ethereum logs and WebSocket streams, ~50 legitimate uses, and gating on it
# would train people to work around the check rather than obey it. A term that
# is genuinely ambiguous belongs in review, not in a gate that cries wolf.
TERMS='billing|stripe'

# Docs and agent instructions are exempt for the same reason they are exempt
# from the ticket-id check: they explain the rule and must be able to name it.
#
# Workflow files are exempt for a sharper version of that: the CI step that
# runs this guard has to NAME what it forbids, and its self-test has to WRITE
# the term in order to prove the guard fires. Without this exemption the guard
# fails on the workflow that invokes it - which it duly did on first run.
#
# The tradeoff, stated rather than buried: a genuine leak inside a workflow
# file would not be caught by the content scan. Accepted because workflows are
# small, reviewed, and carry no product vocabulary - and because the FILENAME
# check below still applies to them, as does the separate branch-name and
# PR-title guard.
EXEMPT='^(CLAUDE\.md|AGENTS\.md|docs/|\.github/workflows/|scripts/check-no-commercial-terms\.sh|scripts/commercial-terms-baseline\.txt)'

if ! files="$(git ls-files)"; then
  echo "::error::git ls-files failed - cannot scan for commercial terms" >&2
  exit 1
fi

# A path counts too: a filename is as public as its contents, and the leak we
# found included `settings_billing_store.sql`.
current=""
while IFS= read -r f; do
  [ -z "$f" ] && continue
  printf '%s' "$f" | grep -qE "$EXEMPT" && continue
  n=0
  if printf '%s' "$f" | grep -qiE "$TERMS"; then n=$((n+1)); fi
  if [ -f "$f" ]; then
    c="$(grep -icE "$TERMS" -- "$f" 2>/dev/null || true)"
    [ -n "$c" ] && n=$((n+c))
  fi
  [ "$n" -gt 0 ] && current="${current}${f} ${n}"$'\n'
done <<< "$files"

status=0
total=0
while IFS=' ' read -r f n; do
  [ -z "$f" ] && continue
  total=$((total+n))
  allowed=0
  if [ -f "$BASELINE" ]; then
    a="$(awk -v want="$f" '$1 == want { print $2; exit }' "$BASELINE")"
    [ -n "$a" ] && allowed="$a"
  fi
  if [ "$n" -gt "$allowed" ]; then
    status=1
    if [ "$allowed" -eq 0 ]; then
      echo "::error::$f introduces a commercial term ($n occurrence(s)). This repository is public; billing and subscription semantics belong in the private repository." >&2
    else
      echo "::error::$f went from $allowed to $n commercial-term occurrence(s). The baseline is a debt to pay down, not a budget to spend." >&2
    fi
  fi
done <<< "$current"

# COMMIT MESSAGES ARE AS PUBLIC AS SOURCE AND INVISIBLE TO THE SCAN ABOVE.
# A guard that reads only the diff would have passed the PR that leaked, since
# its disclosure was a title, a branch name and a subject line. Scans the
# range rather than all history: the backlog is already public and permanent,
# so what matters is what a change ADDS. Mirrors check-no-session-urls.sh,
# which scans commits for the same reason.
BASE_REF="${BASE_REF:-origin/testnet}"
if git rev-parse --verify --quiet "$BASE_REF" >/dev/null 2>&1; then
  range="${BASE_REF}..HEAD"
elif git rev-parse --verify --quiet "HEAD~1" >/dev/null 2>&1; then
  echo "::warning::$BASE_REF not available; checking the last commit only" >&2
  range="HEAD~1..HEAD"
else
  range=""
fi
if [ -n "$range" ]; then
  while read -r sha; do
    [ -z "$sha" ] && continue
    if git log -1 --format='%B' "$sha" | grep -qiE "$TERMS"; then
      status=1
      echo "::error::commit ${sha:0:7} uses a commercial term in its message: $(git log -1 --format='%s' "$sha")" >&2
      echo "  A commit message is public and cannot be edited once pushed to a shared branch." >&2
    fi
  done <<< "$(git log --format='%H' "$range" 2>/dev/null)"
fi

echo "commercial-term occurrences: $total (baseline permits the recorded set only)"
if [ "$status" -ne 0 ]; then
  echo "  Move the work to the private billing repository, or rename the concept." >&2
  echo "  Closing a public PR does not retract its title, branch or diff." >&2
fi
exit "$status"
