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
# guard and watch the tests go red before trusting them. A test that merely
# names two handlers counts for both, and the `Some(scope)` need not be the
# argument to that handler. Fns are keyed by bare name: a name defined in more
# than one non-test file is refused as ambiguous rather than guessed at, and
# `.method(` calls are never counted as callers. Inline test modules are
# `#[cfg(test)] mod NAME {` blocks; any other `#[cfg(test)]` is ignored.
#
# git ls-files, not a bare recursive grep: grep on this box is ugrep and
# honours .gitignore, which has already produced a false-clean scan.
set -uo pipefail

cd "${SCOPE_GUARD_ROOT:-$(git rev-parse --show-toplevel)}" || exit 1

if ! files="$(git ls-files -- '*.rs')"; then
  echo "::error::git ls-files failed while enumerating Rust sources" >&2
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

def code(l):
    return "" if l.lstrip().startswith("//") else l

def indent(l):
    return len(l) - len(l.lstrip())

def split_inline_tests(lines):
    """(production lines with test modules blanked, the lines of the test modules).

    Only a `#[cfg(test)]` that introduces a `mod NAME {` block is a test module,
    and it ends at the closing brace at the same indent. A `#[cfg(test)]` on a
    helper, a `use` or an `impl` mid-file therefore hides nothing after it.
    """
    prod, test = list(lines), []
    i = 0
    while i < len(lines):
        if lines[i].strip() == "#[cfg(test)]":
            k = i + 1
            while k < len(lines) and (not lines[k].strip() or lines[k].lstrip().startswith("#[")):
                k += 1
            if k < len(lines) and re.match(r"\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{\s*$", lines[k]):
                ind, end = indent(lines[k]), k + 1
                while end < len(lines) and not (indent(lines[end]) == ind and lines[end].lstrip().startswith("}")):
                    end += 1
                for x in range(i, min(end + 1, len(lines))):
                    test.append(lines[x])
                    prod[x] = ""
                i = end
        i += 1
    return prod, test

prod, tests, elsewhere = {}, {}, []
for p in sys.stdin.read().split():
    lines = open(p, encoding="utf-8").read().split("\n")
    if is_test_file(p):
        tests[p] = lines
    elif not p.startswith("server/"):
        # Handlers live in server/. A helper called from another crate would be
        # invisible here, so it is reported rather than silently skipped.
        prod_lines, _ = split_inline_tests(lines)
        for i, l in enumerate(prod_lines):
            if any(re.search(r"(?<![.\w])%s\s*(?:::<[^>]*>)?\(" % h, code(l)) for h in HELPERS):
                elsewhere.append("%s:%d  scope helper called outside server/, which this guard does not scan" % (p, i + 1))
    else:
        prod[p], inline = split_inline_tests(lines)
        if inline:
            tests[p + " (inline tests)"] = inline

# Every fn in non-test code: name -> body text; and the fn enclosing each line.
pub = set()
home = {}
defs = {}  # fn name -> files defining it in non-test code
sites = []  # (helper, file, lineno, enclosing fn)
for p, lines in prod.items():
    cur = None
    for i, raw in enumerate(lines):
        l = code(raw)
        m = FN.match(l)
        if m:
            cur = m.group(2)
            home.setdefault(cur, p)
            defs.setdefault(cur, set()).add(p)
            if m.group(1):
                pub.add(cur)
        for h in HELPERS:
            for mm in re.finditer(r"(?<![.\w])%s\s*(?:::<[^>]*>)?\(" % h, l):
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
            ind = indent(lines[i])
            while j < len(lines) and not (indent(lines[j]) == ind and lines[j].lstrip().startswith("}")):
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

def calls(fn, l):
    """True if l calls the free fn `fn`: not a method call, and not `Type::fn(`."""
    for m in re.finditer(r"(?<![.\w])((?:\w+::)*)%s\s*(?:::<[^>]*>)?\(" % fn, l):
        segs = [x for x in m.group(1).split("::") if x]
        if not segs or not segs[-1][:1].isupper():
            return True
    return False

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
            if cur and cur != fn and calls(fn, l):
                out.add(cur)
    return out

def missing(fn, seen):
    """(handler, direction) pairs still unproven for fn, descending into callers.

    The handler is the outermost caller reached, so a baseline entry names the
    handler and never silences a caller added to a helper later.
    """
    if len(defs.get(fn, ())) > 1:
        return [(fn, "unambiguous name (defined in %s)" % ", ".join(sorted(defs[fn])))]
    g, r = directions(fn)
    if g and r:
        return []
    up = callers(fn) - seen
    if not up:
        return [(fn, d) for d, ok in (("grant", g), ("refusal", r)) if not ok]
    seen = seen | {fn}
    out = []
    for c in sorted(up):
        out += missing(c, seen)
    return out

try:
    baseline = {l.split("#")[0].strip() for l in open(BASELINE, encoding="utf-8")} - {""}
except FileNotFoundError:
    baseline = set()

bad, gapped = list(elsewhere), set()
for h, p, ln, fn in sites:
    if fn is None:
        bad.append("%s:%d  %s called outside any fn" % (p, ln, h))
        continue
    for leaf, d in sorted(set(missing(fn, set()))):
        if leaf in baseline:
            gapped.add(leaf)
        else:
            bad.append("%s:%d  %s in `%s` - handler `%s` has no test driving: %s" % (p, ln, h, fn, leaf, d))

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
