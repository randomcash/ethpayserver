#!/usr/bin/env bash
# Refuse a commons pin that is not on payserver-commons main.
#
# A cross-repo ticket pins commons by branch sha before the commons PR merges.
# Merge ethpayserver first (or forget to re-pin) and the workspace ends up
# pinned to a commit that lives only on the commons branch - and once that
# branch is deleted by a squash merge, the commit is gone for good. Nothing
# fails at merge time: the build stays green until the next clean checkout
# tries to resolve the pin and finds nothing there.
#
# Commons is public, so this needs no token - an anonymous, blobless clone of
# `main` is enough to walk its commit graph.
#
# Must-fail-first, run against real payserver-commons commits (2026-09-14):
#   - pin set to 1319b2a7b018397a23afc6f157b791a699590eb3 (tip of
#     origin/rcs/rcs-217-feat-merchants-must-explicitly-acknowled, not merged
#     to main) -> exit 1, "is not on commons main".
#   - pin set to 780dd224d5f756901f45efe68fdd5bb4c7f416ff (this repo's actual
#     pin, on main) -> exit 0, "is on payserver-commons main".
#   - one crate line left on the branch-only sha while the rest were reverted
#     to the main sha (partial re-pin) -> exit 1, reporting only the
#     offending rev and "ok" for the rest.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/Cargo.toml"
url="https://github.com/randomcash/payserver-commons.git"

# Every `rev = "..."` on a payserver-commons line, deduped. One revision is
# shared by all crates in practice, but read them all rather than assume it -
# a PR that moves only some lines is exactly the mistake worth catching.
revs="$(grep 'payserver-commons\.git' "$manifest" | grep -oP 'rev = "\K[0-9a-f]{40}' | sort -u)"
[ -n "$revs" ] || { echo "::error::no payserver-commons rev found in Cargo.toml"; exit 1; }

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "fetching payserver-commons main..."
# --single-branch: only main's history, not every branch in the repo.
# --filter=blob:none: commits and trees, not file contents - all a commit-graph
# walk needs. No --depth: a shallow clone truncates the graph, so a pin older
# than the truncation point has no local object to check against and reads as
# "not on main" - failing the build over a perfectly good, merged pin.
git clone -q --single-branch --branch main --filter=blob:none --no-checkout "$url" "$workdir/commons"

fail=0
while IFS= read -r rev; do
  if git -C "$workdir/commons" merge-base --is-ancestor "$rev" origin/main 2>/dev/null; then
    echo "ok: $rev is on payserver-commons main"
  else
    fail=1
    echo "::error::payserver-commons rev $rev is not on commons main."
    echo "  Merge the commons PR first, then re-pin with: scripts/commons.sh pin <sha>"
  fi
done <<< "$revs"

exit "$fail"
