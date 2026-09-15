#!/usr/bin/env bash
# Proves check-migrations.sh goes red on the cases it claims to, against a real
# throwaway git repository - not a comment asserting someone ran it once.
#
# The guard exists to catch a collision that is invisible until a deploy, so
# "it passed on a tree that was already fine" is not evidence it works.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-migrations.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
check() { # name expected_rc
  local name="$1" want="$2" got
  ( cd "$TMP" && MIGRATIONS_DIR=m "$GUARD" ) >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"
    fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

cd "$TMP"
git init -q .
git config user.email t@t; git config user.name t
mkdir -p m

# Baseline: two distinct versions, both well-formed.
echo "SELECT 1;" > m/20260101000001_first.sql
echo "SELECT 1;" > m/20260101000002_second.sql
git add -A && git commit -qm base
check "distinct versions pass" 0

# The real incident: a second migration claiming a taken version.
echo "SELECT 2;" > m/20260101000002_collides.sql
git add -A && git commit -qm collide
check "duplicate version is refused" 1

git rm -q m/20260101000002_collides.sql && git commit -qm uncollide
check "green again once renamed" 0

# A name sqlx cannot parse a version from is silently never run.
echo "SELECT 3;" > m/not_a_migration.sql
git add -A && git commit -qm malformed
check "unparseable filename is refused" 1
git rm -q m/not_a_migration.sql && git commit -qm clean

# A down-migration shares its version with its up by design.
echo "DROP TABLE x;" > m/20260101000002_second.down.sql
git add -A && git commit -qm down
check "down-migration is not a collision" 0

[ "$fail" -eq 0 ] && echo "check-migrations.sh behaves as documented"
exit "$fail"
