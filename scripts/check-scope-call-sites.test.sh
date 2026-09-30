#!/usr/bin/env bash
# Proves check-scope-call-sites.sh goes red on the cases it claims to, against a
# real throwaway git repository - a guard nobody has seen refuse anything is
# the same defect it exists to catch.
set -uo pipefail

GUARD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-scope-call-sites.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail=0
# A red case must be red for the stated reason: a crash also exits 1, so the
# refusal message is matched, not just the exit code.
check() { # name expected_rc [stderr-substring]
  local name="$1" want="$2" needle="${3:-}" got out
  out="$( cd "$TMP" && SCOPE_GUARD_ROOT="$TMP" SCOPE_GUARD_BASELINE="$TMP/baseline" "$GUARD" 2>&1 >/dev/null )"
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name - expected exit $want, got $got"; echo "$out" | sed 's/^/    /'
    fail=1
  elif [ -n "$needle" ] && ! grep -qF -- "$needle" <<<"$out"; then
    echo "FAIL: $name - exit $got but stderr lacks: $needle"; echo "$out" | sed 's/^/    /'
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
check "handler with no tests is refused" 1 'handler `list_things` has no test driving: grant'

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
check "grant test alone is refused" 1 'handler `list_things` has no test driving: refusal'

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
check "refusal test alone is refused" 1 'handler `list_things` has no test driving: grant'

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
check "tests that pass no Some(scope) do not count" 1 'handler `change_thing` has no test driving: grant'

sed -i 's/change_thing(None)/change_thing(Some(vec!["p".to_string()]))/' server/tests/t.rs
commit both
check "both directions pass, helper covered through its caller" 0

echo "inner" > baseline
commit stale
check "baseline entry that is now covered is refused" 1 'baseline lists `inner`'

: > baseline
sed -i '/is_refused_change_thing/,$d' server/tests/t.rs
sed -i '$d' server/tests/t.rs
# Keyed on the handler, not on the helper it reaches through.
echo "change_thing" > baseline
commit baselined
check "a baselined gap is tolerated" 0

printf 'pub async fn other_thing(s: Option<&[String]>) {\n    inner(s).await;\n}\n' >> server/src/handler.rs
commit newcaller
check "a new caller of a baselined helper is still refused" 1 'handler `other_thing` has no test driving'

: > baseline
echo "pub async fn new_one(s: Option<&[String]>) { narrow_scope_by_key(a, s, V)?; }" >> server/src/handler.rs
commit newone
check "a new undriven handler is refused" 1 'handler `new_one` has no test driving'

# Inline test modules: indented fns, two tests, one per direction.
git rm -q -r server/tests server/src/handler.rs; mkdir -p server/src; : > baseline
cat > server/src/inline.rs <<'RS'
pub async fn inline_thing(scope: Option<&[String]>) {
    let scope = narrow_scope_by_key(all, scope, VIEW)?;
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_key_scoped_to_view_can_inline_thing() {
        inline_thing(Some(vec!["p".to_string()]));
    }

    #[tokio::test]
    async fn a_key_scoped_to_other_is_refused_inline_thing() {
        inline_thing(Some(vec!["p".to_string()]));
    }
}
RS
commit inline
check "indented inline tests count in both directions" 0

# The first test must not swallow the rest of the module.
sed -i 's/is_refused_inline_thing/is_ignored_inline_thing/' server/src/inline.rs
commit inline-one-direction
check "inline module with one direction is refused" 1 'handler `inline_thing` has no test driving: refusal'

# A mid-file #[cfg(test)] on a helper must not hide the handlers below it.
cat > server/src/mid.rs <<'RS'
#[cfg(test)]
fn test_only_helper() {}

pub async fn hidden_thing(scope: Option<&[String]>) {
    let scope = narrow_scope_by_key(all, scope, VIEW)?;
}
RS
commit mid
check "cfg(test) on a helper does not hide later handlers" 1 'handler `hidden_thing` has no test driving'

# A storage method sharing a covered handler name is not a caller of it.
git rm -q server/src/mid.rs; mkdir -p server/src
cat > server/src/store.rs <<'RS'
pub async fn covered_thing(scope: Option<&[String]>) {
    let scope = narrow_scope_by_key(all, scope, VIEW)?;
}

pub async fn some_handler(scope: Option<&[String]>) {
    let rows = repo.covered_thing(scope).await;
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_key_scoped_to_view_can_some_handler() {
        some_handler(Some(vec!["p".to_string()]));
    }

    #[tokio::test]
    async fn a_key_scoped_to_other_is_refused_some_handler() {
        some_handler(Some(vec!["p".to_string()]));
    }
}
RS
sed -i 's/is_ignored_inline_thing/is_refused_inline_thing/' server/src/inline.rs
commit method
check "a .method() call is not counted as a caller" 1 'handler `covered_thing` has no test driving'

# Two files defining the same fn name cannot share one set of tests.
git rm -q server/src/store.rs; mkdir -p server/src
echo "pub async fn inline_thing() {}" > server/src/dup.rs
commit dup
check "a fn name defined in two files is refused as ambiguous" 1 "unambiguous name"

[ "$fail" -eq 0 ] && echo "check-scope-call-sites.sh behaves as documented"
exit "$fail"
