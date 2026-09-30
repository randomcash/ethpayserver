#!/usr/bin/env bash
# Proves check-scope-call-sites.sh goes red on the cases it claims to, against a
# real throwaway git repository - a guard nobody has seen refuse anything is
# the same defect it exists to catch.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-scope-call-sites.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
check() { # name expected_rc
  local name="$1" want="$2" got
  ( cd "$TMP" && SCOPE_GUARD_ROOT="$TMP" SCOPE_GUARD_BASELINE="$TMP/baseline" "$GUARD" ) >/dev/null 2>&1
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
mkdir -p server/src server/tests

cat > server/src/handler.rs <<'RS'
pub async fn list_things(scope: Option<&[String]>) {
    let scope = narrow_scope_by_key(all, scope, VIEW)?;
}

async fn inner(scope: Option<&[String]>) {
    require_store_settings_permission(a, b, scope, c).await?;
}

pub async fn change_thing(scope: Option<&[String]>) {
    inner(scope).await;
}
RS
: > baseline
commit() { git add server baseline && git commit -qm "$1"; }
commit base
check "handler with no tests is refused" 1

cat > server/tests/t.rs <<'RS'
#[tokio::test]
async fn a_key_scoped_to_view_can_list_things() {
    list_things(Some(vec!["p".to_string()]));
}

#[tokio::test]
async fn a_key_scoped_to_view_can_change_thing() {
    change_thing(Some(vec!["p".to_string()]));
}
RS
commit grant-only
check "grant test alone is refused" 1

cat > server/tests/t.rs <<'RS'
#[tokio::test]
async fn a_key_scoped_to_other_is_refused_list_things() {
    list_things(Some(vec!["p".to_string()]));
}

#[tokio::test]
async fn a_key_scoped_to_other_is_refused_change_thing() {
    change_thing(Some(vec!["p".to_string()]));
}
RS
commit refuse-only
check "refusal test alone is refused" 1

cat > server/tests/t.rs <<'RS'
#[tokio::test]
async fn a_key_scoped_to_view_can_list_things() {
    list_things(Some(vec!["p".to_string()]));
}

#[tokio::test]
async fn a_key_scoped_to_other_is_refused_list_things() {
    list_things(Some(vec!["p".to_string()]));
}

#[tokio::test]
async fn a_key_scoped_to_view_can_change_thing() {
    change_thing(None);
}

#[tokio::test]
async fn a_key_scoped_to_other_is_refused_change_thing() {
    change_thing(None);
}
RS
commit unscoped
check "tests that pass no Some(scope) do not count" 1

sed -i 's/change_thing(None)/change_thing(Some(vec!["p".to_string()]))/' server/tests/t.rs
commit both
check "both directions pass, helper covered through its caller" 0

echo "inner" > baseline
commit stale
check "baseline entry that is now covered is refused" 1

: > baseline
sed -i '/is_refused_change_thing/,$d' server/tests/t.rs
sed -i '$d' server/tests/t.rs
# Keyed on the fn enclosing the call site, which for a helper is the helper.
echo "inner" > baseline
commit baselined
check "a baselined gap is tolerated" 0

: > baseline
echo "pub async fn new_one(s: Option<&[String]>) { narrow_scope_by_key(a, s, V)?; }" >> server/src/handler.rs
commit newone
check "a new undriven handler is refused" 1

[ "$fail" -eq 0 ] && echo "check-scope-call-sites.sh behaves as documented"
exit "$fail"
