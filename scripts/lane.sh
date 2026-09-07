#!/usr/bin/env bash
# Create an isolated "lane" for parallel work: a git worktree of this repo with
# a matching worktree of payserver-commons sitting beside it.
#
# The pairing is the whole point. The root Cargo.toml patches the commons crates
# by RELATIVE path:
#
#     [patch."https://github.com/randomcash/payserver-commons.git"]
#     types = { path = "../payserver-commons/types" }
#
# so a checkout only builds when payserver-commons is its immediate sibling. A
# worktree created anywhere else - `.claude/worktrees/`, /tmp, a subdirectory -
# resolves `../payserver-commons` to nothing and the build dies before it starts.
# Pairing them per lane also means two lanes can sit on different commons
# branches, which a single shared checkout cannot do.
#
# Usage:
#   scripts/lane.sh <lane-name> [base-branch]     create (default base: testnet)
#   scripts/lane.sh --list
#   scripts/lane.sh --remove <lane-name>
#
# Layout:
#   <parent>/lanes/<lane-name>/ethpayserver-cockpit   <- work here
#   <parent>/lanes/<lane-name>/payserver-commons
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
parent="$(dirname "$repo_root")"
commons="$parent/payserver-commons"
lanes="$parent/lanes"

die() { echo "error: $*" >&2; exit 1; }

case "${1:-}" in
  --list)
    echo "== $(basename "$repo_root")"; git -C "$repo_root" worktree list
    [ -d "$commons" ] && { echo; echo "== payserver-commons"; git -C "$commons" worktree list; }
    exit 0 ;;
  --remove)
    name="${2:?usage: lane.sh --remove <name>}"
    dir="$lanes/$name"
    [ -d "$dir" ] || die "no lane '$name' at $dir"
    # --force because a lane with a dirty tree is the normal case; the branch
    # survives either way, so nothing committed is lost.
    git -C "$repo_root" worktree remove --force "$dir/$(basename "$repo_root")" 2>/dev/null || true
    git -C "$commons"   worktree remove --force "$dir/payserver-commons"       2>/dev/null || true
    rmdir "$dir" 2>/dev/null || true
    echo "removed lane '$name'"
    exit 0 ;;
  "" | -h | --help)
    sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'
    exit 0 ;;
esac

name="$1"
base="${2:-testnet}"
[[ "$name" =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "lane name must be kebab-case: '$name'"
[ -d "$commons" ] || die "expected payserver-commons beside this repo at $commons"

dir="$lanes/$name"
[ -e "$dir" ] && die "lane '$name' already exists at $dir"
mkdir -p "$dir"

# One branch name across both repos. CI clones commons by branch name and falls
# back to its default, so matching names are what make a paired change build
# together on the runner as well as locally.
branch="lane/$name"

git -C "$repo_root" fetch -q origin
git -C "$repo_root" worktree add -q -b "$branch" "$dir/$(basename "$repo_root")" "origin/$base"

# commons uses `main`; only create a branch there if the lane will touch it.
# Always making one leaves noise behind for lanes that never needed it.
git -C "$commons" fetch -q origin
git -C "$commons" worktree add -q --detach "$dir/payserver-commons" origin/main

cat <<EOF
lane '$name' ready

  work in : $dir/$(basename "$repo_root")
  branch  : $branch  (from origin/$base)
  commons : $dir/payserver-commons  (detached at origin/main)

If this lane changes commons, give it the same branch name so CI builds both:
  git -C "$dir/payserver-commons" switch -c "$branch"

When done:
  scripts/lane.sh --remove $name
EOF
