#!/usr/bin/env bash
# Refuses a migration set that sqlx would reject at runtime.
#
# sqlx keys applied migrations by the numeric version in the filename, in a
# table it writes on first apply. Two files sharing a version is therefore not
# a naming nit: the first one to run records that version, the second finds a
# row already there, compares checksums, and returns VersionMismatch - on that
# deploy and on every startup after it, because the version stays recorded.
# Recovering means editing the database by hand.
#
# It is easy to produce. Two branches opened the same afternoon pick the same
# YYYYMMDDnnnnnn stamp, each is green alone, and the collision only exists once
# both have merged. That happened on 2026-09-15: two branches both claimed
# version 20260914000001, and nothing would have objected until a deploy did.
#
# git ls-files, not a glob: an untracked scratch migration in the working tree
# is not what deploys, and a shell glob would fail this check on it.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

DIR="${MIGRATIONS_DIR:-data-service/migrations/postgres}"
status=0

# `--others` as well as the index, because the file that introduces a
# collision is by definition a new one. `git ls-files` alone sees only what is
# already tracked, so running this straight after writing a migration - the
# one moment it is worth running - reported "all distinct" about a directory
# containing two copies of the same version. CI never saw the gap, since by
# then everything is committed.
#
# `--exclude-standard` keeps .gitignore honoured, so build output and editor
# leftovers do not become migrations.
versions="$(git ls-files --cached --others --exclude-standard "$DIR" \
  | grep -E '\.sql$' \
  | grep -vE '\.down\.sql$' \
  | xargs -r -n1 basename \
  | sed -E 's/^([0-9]+)_.*/\1/')"

if [ -z "$versions" ]; then
  echo "::error::no migrations found under $DIR - wrong path?" >&2
  exit 1
fi

# 1. No two up-migrations may share a version.
dupes="$(printf '%s\n' "$versions" | sort | uniq -d)"
if [ -n "$dupes" ]; then
  status=1
  while IFS= read -r v; do
    [ -z "$v" ] && continue
    echo "::error::two migrations share version $v - sqlx will refuse the second one on every startup:" >&2
    git ls-files "$DIR" | grep -E "/${v}_" | sed 's/^/    /' >&2
  done <<< "$dupes"
  echo "  Rename one. The version comes from the filename, so renaming is free;" >&2
  echo "  editing an applied migration's bytes is what is not." >&2
fi

# 2. A filename sqlx cannot parse a version out of is not a migration it will
#    run, and it will not say so.
while IFS= read -r f; do
  [ -z "$f" ] && continue
  base="$(basename "$f")"
  if ! printf '%s' "$base" | grep -qE '^[0-9]+_[^/]+\.sql$'; then
    echo "::error::$f is not <version>_<name>.sql - sqlx will not pick it up" >&2
    status=1
  fi
done <<< "$(git ls-files "$DIR" | grep -E '\.sql$' | grep -vE '\.down\.sql$')"

if [ "$status" -eq 0 ]; then
  echo "migrations ok: $(printf '%s\n' "$versions" | wc -l) versions, all distinct"
fi
exit "$status"
