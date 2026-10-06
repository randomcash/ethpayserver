#!/usr/bin/env bash
# Proves check-test-db-skips.sh goes red on the patterns it claims to, and green
# on a clean tree, against a real throwaway git repository.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-test-db-skips.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
check() { # name expected_rc
  local name="$1" want="$2" got
  ( cd "$TMP" && "$GUARD" ) >/dev/null 2>&1
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"
    fail=1
  else
    echo "ok: $name (exit $got)"
  fi
}

plant() { # name body - writes a tracked file and commits it
  printf '%s\n' "$2" > "$TMP/$1"
  ( cd "$TMP" && git add "$1" && git commit -qm "plant $1" )
}
unplant() { ( cd "$TMP" && git rm -q "$1" && git commit -qm "unplant $1" ); }

cd "$TMP"
git init -q .
git config user.email t@t; git config user.name t
mkdir -p data-service/src
cat > data-service/src/test_support.rs <<'RS'
// The helper itself may read the variable's absence.
pub fn database_url_or_skip() -> Option<String> { std::env::var("DATABASE_URL").ok()? }
RS
echo 'fn fine() { let url = data_service::test_support::database_url(); }' > ok.rs
git add -A && git commit -qm base
check "clean tree passes" 0

plant a.rs 'async fn s() -> Option<X> { let u = std::env::var("DATABASE_URL").ok()?; Some(X) }'
check "DATABASE_URL .ok()? is refused" 1
unplant a.rs

plant b.rs 'async fn s() -> Option<X> {
    let pool = PgPoolOptions::new()
        .connect(&url)
        .await
        .ok()?;
    Some(X)
}'
check "multi-line connect .ok()? is refused" 1
unplant b.rs

plant c.rs 'async fn t() { let Ok(url) = std::env::var("DATABASE_URL") else { return; }; }'
check "let-else skip on DATABASE_URL is refused" 1
unplant c.rs

plant d.rs 'async fn t() { let Some(u) = std::env::var("DATABASE_URL").ok() else { return; }; }'
check "let Some(..) = ..ok() else is refused" 1
unplant d.rs

plant f.rs 'fn t() { match std::env::var("DATABASE_URL") { Err(_) => return, Ok(u) => drop(u) } }'
check "match on DATABASE_URL is refused" 1
unplant f.rs

plant g.rs 'async fn t() { if std::env::var("DATABASE_URL").is_err() { return; } }'
check "is_err() early return on DATABASE_URL is refused" 1
unplant g.rs

plant h.rs 'async fn t() { let Ok(p) = PgPool::connect(&u).await else { return; }; }'
check "connect let-else skip is refused" 1
unplant h.rs

mkdir -p server/src/bin
plant server/src/bin/migrate.rs 'fn main() { let u = std::env::var("DATABASE_URL").unwrap(); }'
check "a listed non-test binary may read DATABASE_URL" 0
unplant server/src/bin/migrate.rs

plant e.rs 'fn x() { let v = "10".parse::<u8>().ok()?; }'
check "unrelated .ok()? is not flagged" 0
unplant e.rs

check "green again once the plants are gone" 0

[ "$fail" -eq 0 ] && echo "check-test-db-skips.sh behaves as documented"
exit "$fail"
