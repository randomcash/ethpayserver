#!/usr/bin/env bash
# Create an isolated "lane" for parallel work: a git worktree of this repo with
# a matching worktree of payserver-commons sitting beside it.
#
# A worktree of this repo builds anywhere now - commons is pinned by revision,
# so nothing depends on directory layout any more. What a lane adds is
# an isolated commons to EDIT.
#
# Without it, every worktree links to the one shared ../payserver-commons on one
# branch, so two lanes touching commons overwrite each other. Each lane gets its
# own checkout and its own `.cargo/config.toml` pointing at it, so the lanes
# cannot interfere - and a lane that never touches commons simply builds the
# pinned revision.
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

# The lane's branch in this repo. Commons no longer needs a matching name: CI
# does not clone it by branch any more, it builds the revision Cargo.toml pins.
branch="lane/$name"

git -C "$repo_root" fetch -q origin
git -C "$repo_root" worktree add -q -b "$branch" "$dir/$(basename "$repo_root")" "origin/$base"

# commons uses `main`; only create a branch there if the lane will touch it.
# Always making one leaves noise behind for lanes that never needed it.
git -C "$commons" fetch -q origin
git -C "$commons" worktree add -q --detach "$dir/payserver-commons" origin/main

# Point the lane at its own commons. The manifest stays pinned; this writes the
# lane's uncommitted .cargo/config.toml so work here builds against the lane's
# checkout rather than the pinned revision.
"$dir/$(basename "$repo_root")/scripts/commons.sh" link "$dir/payserver-commons" >/dev/null

cat <<EOF
lane '$name' ready

  work in : $dir/$(basename "$repo_root")
  branch  : $branch  (from origin/$base)
  commons : $dir/payserver-commons  (detached at origin/main, linked)

If this lane changes commons, matching branch names are NOT enough any more -
CI builds the pinned revision, so a commons branch it never fetches would look
green while compiling none of your change. Land it and move the pin:

  git -C "$dir/payserver-commons" switch -c "$branch"   # work on it
  # merge that in payserver-commons, then, in this lane:
  scripts/commons.sh pin <sha>                          # commit Cargo.toml + Cargo.lock

When done:
  scripts/lane.sh --remove $name
EOF
