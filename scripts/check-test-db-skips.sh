#!/usr/bin/env bash
# Refuses a test helper that skips when the database is down.
#
# A test that needs Postgres is run on purpose (`#[ignore]`d, and CI passes
# --run-ignored). A helper that does `env::var("DATABASE_URL").ok()?`, or
# swallows the connect error with `.connect(..).await.ok()?`, turns an absent
# or unreachable database into a return - and a test that returns reports the
# same "ok" as one that ran. On a day Postgres is slow or missing the suite goes
# green by not existing, and nothing in the output says so.
#
# Every database test connects through data-service/src/test_support.rs, which
# panics naming DATABASE_URL, or - for a skip that is genuinely wanted -
# announces a greppable `SKIPPED:` line. That file is the one place allowed to
# read the variable's absence.
#
# Two rules, both scanned across lines because rustfmt splits a call:
#   1. DATABASE_URL is read nowhere but the helper (and the few non-test
#      binaries listed below). Any other read of it - `.ok()?`, `let ... else`,
#      `match`, `if let Ok`, `.is_err()` then return - is a place a missing
#      database can become a skip, so the shape does not matter.
#   2. A connect whose error is discarded or turned into an early return
#      (`.await.ok()`, `.await.is_err()`, `.await else`).
# It is still a convention check, not a proof: a connect wrapped in a helper of
# its own, or a skip that never reads the variable, would pass it.
#
# git ls-files, not a glob, and not `grep -r`: ugrep honours .gitignore and
# would skip files silently. The scan is multi-line because rustfmt splits
# `.connect(..)` / `.await` / `.ok()?` across lines.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

HELPER="${TEST_DB_HELPER:-data-service/src/test_support.rs}"
status=0

command -v perl >/dev/null || { echo "::error::perl is required by $0" >&2; exit 2; }

# Reads of DATABASE_URL that are not tests: the services and tools that need it
# to run at all, where a missing value is a startup error.
NON_TEST_READS='^(mcp-server/src/main\.rs|server/src/bin/|server/src/config\.rs)'

pattern='(env::var(_os)?\(\s*"DATABASE_URL"\s*\)|connect\([^;]*?\)\s*\.await\s*(\.ok\(\)|\.is_err\(\)|else\b))'

while IFS= read -r f; do
  [ -z "$f" ] && continue
  [ "$f" = "$HELPER" ] && continue
  echo "$f" | grep -qE "$NON_TEST_READS" && continue
  hits="$(perl -0777 -ne '
    my $re = qr/'"$pattern"'/s;
    while (/$re/g) {
      my $line = 1 + (substr($_, 0, $-[0]) =~ tr/\n//);
      print "$line\n";
    }' "$f")" || { echo "::error file=$f::perl failed scanning this file" >&2; status=1; continue; }
  [ -z "$hits" ] && continue
  status=1
  while IFS= read -r line; do
    echo "::error file=$f,line=$line::database test helper skips instead of failing - use data_service::test_support" >&2
  done <<< "$hits"
done < <(git ls-files --cached --others --exclude-standard -- '*.rs')

if [ "$status" -ne 0 ]; then
  echo "  A missing or unreachable DATABASE_URL must panic in a test that is run on purpose." >&2
  echo "  Use data_service::test_support::{pg_service, pg_pool, database_url}, or" >&2
  echo "  database_url_or_skip for a non-ignored test, which prints a SKIPPED: line." >&2
else
  echo "test database helpers ok: none skip silently"
fi
exit "$status"
