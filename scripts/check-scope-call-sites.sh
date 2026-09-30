#!/usr/bin/env bash
# Refuses a call site of an API-key scope helper that no test drives in BOTH
# directions.
#
# A permission check that reads correctly and has nothing exercising it is the
# same defect wherever it appears: `update_store_member` (the risky
# two-path-param handler), `list_payments` and both CSV exports (bulk paths -
# one narrow key downloading a whole store), `create_api_key` (every part
# unit-tested, nothing driving the handler). Each was found by a human read,
# not by a test. This makes the absence a build failure.
#
# Helpers watched: key_grants_store_permission, narrow_scope_by_key,
# require_store_settings_permission.
#
# For each call site in non-test code, the enclosing fn must be covered - or,
# if it is a helper, every caller of it must be (transitively). A fn is
# covered when some test names it in its body AND passes a real `Some(scope)`
# in both:
#   * a granting test  - the test's name says the key is allowed
#                        (`..._can_...`, `..._reaches_past_...`, `..._allowed_...`)
#   * a refusing test  - the test's name says it is refused
#                        (`..._refused_...`, `..._rejected_...`, `..._denied_...`)
# A refusal test alone is not enough: it cannot tell a working guard from a
# handler that refuses everything. A grant test alone cannot tell a guard from
# no guard. Both or it proves nothing.
#
# Limits, stated so nobody mistakes this for more than it is: it is a textual
# check. It proves a test names the handler, passes a scope, and is labelled
# for each direction; it does not prove the assertions are right. Break the
# guard and watch the tests go red before trusting them. Inline test modules
# are assumed to sit at the end of their file, after `#[cfg(test)]`.
#
# git ls-files, not a bare recursive grep: grep on this box is ugrep and
# honours .gitignore, which has already produced a false-clean scan.
set -uo pipefail

cd "${SCOPE_GUARD_ROOT:-$(git rev-parse --show-toplevel)}" || exit 1

if ! files="$(git ls-files -- 'server/*.rs')"; then
  echo "::error::git ls-files failed while enumerating server sources" >&2
  exit 1
fi

printf '%s\n' "$files" | python3 -c '
import os, re, sys

HELPERS = ["key_grants_store_permission", "narrow_scope_by_key",
           "require_store_settings_permission"]
REFUSE = re.compile(r"(^|_)(refus\w*|reject\w*|denied|deny|denies|forbidden|forbids)(_|$)")
GRANT = re.compile(r"(^|_)(can|reaches|allowed|allows|grants|permitted|succeeds)(_|$)")
SCOPE = re.compile(r"Some\(\s*(vec!|&?\w*scope)")
FN = re.compile(r"^\s*(pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)")
BASELINE = os.environ.get("SCOPE_GUARD_BASELINE", "scripts/scope-call-sites.baseline")

def is_test_file(p):
    return ("/tests/" in p or p.endswith("/tests.rs") or p.endswith("_tests.rs")
            or p.startswith("server/tests/"))

prod, tests = {}, {}
for p in sys.stdin.read().split():
    lines = open(p, encoding="utf-8").read().split("\n")
    if is_test_file(p):
        tests[p] = lines
    else:
        cut = next((i for i, l in enumerate(lines) if l.strip() == "#[cfg(test)]"), len(lines))
        prod[p] = lines[:cut]
        if cut < len(lines):
            tests[p + " (inline tests)"] = lines[cut:]

def code(l):
    return "" if l.lstrip().startswith("//") else l

# Every fn in non-test code: name -> body text; and the fn enclosing each line.
pub = set()
home = {}
sites = []  # (helper, file, lineno, enclosing fn)
for p, lines in prod.items():
    cur = None
    for i, raw in enumerate(lines):
        l = code(raw)
        m = FN.match(l)
        if m:
            cur = m.group(2)
            home.setdefault(cur, p)
            if m.group(1):
                pub.add(cur)
        for h in HELPERS:
            for mm in re.finditer(r"\b%s\s*(?:::<[^>]*>)?\(" % h, l):
                if FN.match(l) and FN.match(l).group(2) == h:
                    continue
                # The helpers call each other; their callers are the sites.
                if cur in HELPERS:
                    continue
                sites.append((h, p, i + 1, cur))

# Test fns: (name, body-without-comments), from every test file.
tfns = []
for p, lines in tests.items():
    i = 0
    while i < len(lines):
        m = FN.match(lines[i])
        if m and any(re.match(r"\s*#\[(tokio::)?test", lines[j]) for j in range(max(0, i - 4), i)):
            j = i + 1
            while j < len(lines) and lines[j] != "}":
                j += 1
            tfns.append((m.group(2), "\n".join(code(x) for x in lines[i:j + 1])))
            i = j
        i += 1

def directions(fn):
    grant = refuse = False
    for name, body in tfns:
        if not re.search(r"\b%s\b" % fn, body) or not SCOPE.search(body):
            continue
        if REFUSE.search(name):
            refuse = True
        elif GRANT.search(name):
            grant = True
    return grant, refuse

def callers(fn):
    out = set()
    # A private fn can only be called from its own file; a pub one from anywhere.
    for p, lines in prod.items():
        if fn not in pub and p != home.get(fn):
            continue
        cur = None
        for raw in lines:
            l = code(raw)
            m = FN.match(l)
            if m:
                cur = m.group(2)
            elif cur and cur != fn and re.search(r"\b%s\s*(?:::<[^>]*>)?\(" % fn, l):
                out.add(cur)
    return out

def missing(fn, seen):
    """Directions still unproven for fn, descending into its callers."""
    g, r = directions(fn)
    if g and r:
        return []
    up = callers(fn) - seen
    if not up:
        return [d for d, ok in (("grant", g), ("refusal", r)) if not ok]
    seen = seen | {fn}
    out = []
    for c in sorted(up):
        out += ["%s via %s" % (d, c) for d in missing(c, seen)]
    # A helper is fine if each caller is; report only the callers that are not.
    return out

try:
    baseline = {l.split("#")[0].strip() for l in open(BASELINE, encoding="utf-8")} - {""}
except FileNotFoundError:
    baseline = set()

bad, gapped = [], set()
for h, p, ln, fn in sites:
    if fn is None:
        bad.append("%s:%d  %s called outside any fn" % (p, ln, h))
        continue
    gaps = missing(fn, set())
    if gaps and fn in baseline:
        gapped.add(fn)
    elif gaps:
        bad.append("%s:%d  %s in `%s` - no test drives: %s" % (p, ln, h, fn, "; ".join(sorted(set(gaps)))))

# The baseline only ever shrinks: a handler that gained both tests must leave it.
for fn in sorted(baseline - gapped):
    bad.append("baseline lists `%s` but it is covered or no longer has a call site - remove it from %s" % (fn, BASELINE))

if not sites:
    print("::error::found no call sites of the scope helpers - the guard is scanning nothing", file=sys.stderr)
    sys.exit(1)

if bad:
    print("::error::scope-helper call sites not exercised in both directions:", file=sys.stderr)
    for b in sorted(set(bad)):
        print("  " + b, file=sys.stderr)
    print("""
Each handler that reaches a scope helper needs, in a test that names it and
passes a real Some(scope):
  - a granting test  (test name contains can / reaches_past / allowed), and
  - a refusing test  (test name contains refused / rejected / denied).
A refusal test alone cannot tell a working guard from a handler that refuses
everything; a grant test alone cannot tell a guard from no guard.""", file=sys.stderr)
    sys.exit(1)

print("%d scope-helper call sites: none newly undriven (%d handlers still on the baseline)" % (len(sites), len(gapped)))
'
