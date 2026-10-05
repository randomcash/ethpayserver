#!/usr/bin/env bash
# Proves check-commons-pin.sh actually goes red and green on the cases it
# claims to guard, instead of asserting that in a comment. A guard whose
# red/green behaviour was only ever described by its own author is exactly
# the shape this ticket exists to prevent - see the guard's own header.
#
# Builds a real, throwaway commons history (one commit on `main`, a second on
# an unmerged branch) and clones from it over `file://`, so the guard walks
# real git objects rather than hex strings a test could get away with faking.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$repo_root/scripts/check-commons-pin.sh"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

commons="$workdir/commons"
git init -q -b main "$commons"
git -C "$commons" -c user.email=t@t -c user.name=t commit -q --allow-empty -m main
main_sha="$(git -C "$commons" rev-parse HEAD)"
git -C "$commons" -c user.email=t@t -c user.name=t checkout -q -b feature
git -C "$commons" -c user.email=t@t -c user.name=t commit -q --allow-empty -m feature
branch_sha="$(git -C "$commons" rev-parse HEAD)"
git -C "$commons" checkout -q main

# check-commons-pin.sh resolves its own repo root from its script path (one
# directory up from itself), so a copy placed at fixture/scripts/ reads
# fixture/Cargo.toml - a synthetic manifest, with no risk of ever touching
# this repo's real one.
fixture="$workdir/fixture"
mkdir -p "$fixture/scripts"
cp "$guard" "$fixture/scripts/check-commons-pin.sh"

pass=0
fail=0

run_guard() {
  COMMONS_URL="file://$commons" COMMONS_CLONE_TIMEOUT=10 "$fixture/scripts/check-commons-pin.sh" \
    >"$workdir/out" 2>&1
}

write_manifest() {
  printf '[dependencies]\n' > "$fixture/Cargo.toml"
  for rev in "$@"; do
    printf 'payserver-api-types-%s = { git = "https://github.com/randomcash/payserver-commons.git", rev = "%s" }\n' \
      "$rev" "$rev" >> "$fixture/Cargo.toml"
  done
}

assert_exit() {
  local desc="$1" want="$2"
  shift 2
  write_manifest "$@"
  local rc=0
  run_guard || rc=$?
  if [ "$rc" -eq "$want" ]; then
    echo "ok: $desc (exit $rc)"
    pass=$((pass + 1))
  else
    echo "FAIL: $desc - wanted exit $want, got $rc"
    sed 's/^/  /' "$workdir/out"
    fail=$((fail + 1))
  fi
}

assert_exit "branch-only sha is rejected" 1 "$branch_sha"
assert_exit "main sha is accepted" 0 "$main_sha"
assert_exit "short sha is rejected without needing a clone" 1 "${main_sha:0:7}"

# The motivating incident: one crate line re-pinned to main, one left behind
# on the branch-only sha. The guard must still catch the leftover line.
write_manifest "$main_sha" "$branch_sha"
rc=0
run_guard || rc=$?
if [ "$rc" -eq 1 ] && grep -q "$branch_sha" "$workdir/out" && grep -q "ok: $main_sha" "$workdir/out"; then
  echo "ok: partial re-pin still catches the leftover branch sha"
  pass=$((pass + 1))
else
  echo "FAIL: partial re-pin - wanted exit 1 naming $branch_sha and ok for $main_sha, got exit $rc"
  sed 's/^/  /' "$workdir/out"
  fail=$((fail + 1))
fi

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
