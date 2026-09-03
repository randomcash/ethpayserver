#!/bin/bash
# Tag and publish an ethpayserver release.
#
# Exists because a template you can skip is a template you will skip. The
# "Not ready for" section is the one that earns the release notes their
# credibility, and it is also the first thing dropped when a release feels
# routine — so this refuses to tag without it.
#
# Checks, in order:
#   1. RELEASE_NOTES_<version>.md exists
#   2. It has a "## Not ready for" section that is non-empty and free of
#      template placeholders
#   3. Anything not matching vX.Y.Z exactly is published as a pre-release,
#      so -alpha/-beta/-rc cannot be *labelled* stable by accident
#   4. The working tree is clean and the tag does not already exist
#   5. The deployed build matches the commit being tagged (skippable)
#
# WARNING - tagging deploys. `gh release create` pushes the tag, and
# .github/workflows/ci.yml triggers on `tags: ["v*"]` and resolves ANY v* tag
# to environment=production, dispatching a deploy to central-infrastructure.
# -alpha is not exempt: --prerelease changes the label on the releases page,
# not where the code goes. Every tag is therefore confirmed interactively, and
# --dry-run runs every check and stops before the tag is created.
#
# Usage:
#   scripts/release.sh --dry-run v0.2.0-alpha
#   scripts/release.sh v0.2.0-alpha
#   SKIP_DEPLOY_CHECK=1 scripts/release.sh v0.2.0-alpha
#
# Env:
#   TARGET_BRANCH       branch to tag (default: testnet)
#   HEALTH_URL          deployment to check build_sha against
#                       (default: https://testnet.random.cash/api/health)
#   SKIP_DEPLOY_CHECK   set to skip check 5

set -euo pipefail

DRY_RUN=""
if [ "${1:-}" = "--dry-run" ]; then
    DRY_RUN=1
    shift
fi

VERSION="${1:-}"
TARGET_BRANCH="${TARGET_BRANCH:-testnet}"
HEALTH_URL="${HEALTH_URL:-https://testnet.random.cash/api/health}"

die() { echo "error: $*" >&2; exit 1; }

[ -n "$VERSION" ] || die "usage: scripts/release.sh [--dry-run] vX.Y.Z[-alpha]"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-z0-9.]+)?$ ]] \
    || die "version must look like v1.2.3 or v1.2.3-alpha, got: $VERSION"

NOTES="RELEASE_NOTES_${VERSION}.md"

# --- 1. notes exist -----------------------------------------------------------
[ -f "$NOTES" ] || die "$NOTES not found. Start from .github/RELEASE_TEMPLATE.md"

# --- 2. limitations section is real -------------------------------------------
# Everything from the heading to the next one, comments and blanks stripped.
#
# The comment stripping is stateful on purpose. `sed 's/<!--.*-->//'` only
# handles comments that open and close on one line, and dropping lines that
# *start* with <!-- or --> leaves the interior of a block behind. The template's
# own "Not ready for" guidance is a multi-line block, so an author who deletes
# the placeholder bullets and leaves the comment used to pass this check with
# $LIMITS holding nothing but the template's instructions to themselves - the
# section empty, the guard satisfied, the release published.
LIMITS=$(awk '
    /^## Not ready for/ { grab = 1; next }
    /^## /              { grab = 0 }
    !grab               { next }
    {
        line = $0
        while (1) {
            if (in_comment) {
                close_at = index(line, "-->")
                if (close_at == 0) { line = ""; break }
                line = substr(line, close_at + 3)
                in_comment = 0
            } else {
                open_at = index(line, "<!--")
                if (open_at == 0) break
                before = substr(line, 1, open_at - 1)
                rest   = substr(line, open_at + 4)
                close_at = index(rest, "-->")
                if (close_at == 0) { line = before; in_comment = 1; break }
                line = before substr(rest, close_at + 3)
            }
        }
        if (line ~ /[^[:space:]]/) print line
    }
' "$NOTES")

[ -n "$LIMITS" ] \
    || die "$NOTES has no '## Not ready for' section, or it is empty.
       This is required on every release including alphas and patches. If a
       release genuinely has no known gaps, say so explicitly — that is a
       claim worth making deliberately rather than by omission."

if echo "$LIMITS" | grep -qE 'Known gap, with the ticket|What a user should not attempt yet'; then
    die "$NOTES still contains template placeholder text under 'Not ready for'."
fi

if ! echo "$LIMITS" | grep -qE '(RCS-[0-9]+|#[0-9]+)'; then
    echo "warning: no ticket references under 'Not ready for'." >&2
    echo "         'Recovery has no UI (RCS-205)' is checkable; 'some features" >&2
    echo "         are incomplete' is noise. Continuing anyway." >&2
fi

# --- 3. pre-release unless the version is exactly vX.Y.Z ----------------------
PRERELEASE_FLAG="--prerelease"
if [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    PRERELEASE_FLAG=""
    echo "This is a STABLE release ($VERSION), not a pre-release."
    echo "For a payment processor that asserts production readiness for real funds."
else
    echo "This is a pre-release ($VERSION)."
fi

# Confirm unconditionally: --prerelease only affects the label on the releases
# page. The tag itself dispatches a production deploy either way, so a
# pre-release is not the safer path it reads as.
echo "Tagging dispatches a PRODUCTION deploy (ci.yml maps every v* tag to it)."
if [ -n "$DRY_RUN" ]; then
    echo "dry run: skipping confirmation."
else
    read -r -p "Type the version again to confirm: " CONFIRM
    [ "$CONFIRM" = "$VERSION" ] || die "confirmation did not match; nothing tagged"
fi

# --- 4. clean tree, unused tag ------------------------------------------------
[ -z "$(git status --porcelain)" ] || die "working tree is dirty; commit or stash first"
git fetch --quiet origin
! git rev-parse "$VERSION" >/dev/null 2>&1 || die "tag $VERSION already exists"

TARGET_SHA=$(git rev-parse --short "origin/$TARGET_BRANCH")
echo "target: origin/$TARGET_BRANCH @ $TARGET_SHA"

# --- 5. deployed build matches what we are tagging ----------------------------
if [ -z "${SKIP_DEPLOY_CHECK:-}" ]; then
    # Parsed as JSON, like scripts/health-gate.sh - a string grep breaks the
    # moment the endpoint pretty-prints or adds a space after the colon, and it
    # breaks by returning empty, which is indistinguishable from "unreachable".
    DEPLOYED=$(curl -fsS --max-time 15 "$HEALTH_URL" 2>/dev/null \
        | python3 -c 'import json,sys; print(json.load(sys.stdin).get("build_sha",""))' 2>/dev/null || true)
    if [ -z "$DEPLOYED" ]; then
        die "could not read build_sha from $HEALTH_URL.
       This check is what stops a release pointing at code nobody has run, so
       it fails closed. Re-run with SKIP_DEPLOY_CHECK=1 if you have confirmed
       the deployment another way."
    elif [ "$DEPLOYED" != "$TARGET_SHA" ]; then
        die "deployed build is $DEPLOYED but you are tagging $TARGET_SHA.
       The release would point at code nobody has run. Wait for the deploy,
       or re-run with SKIP_DEPLOY_CHECK=1 if that is deliberate."
    else
        echo "deployed build matches ($DEPLOYED)"
    fi
fi

# --- publish ------------------------------------------------------------------
echo
if [ -n "$DRY_RUN" ]; then
    echo "dry run: all checks passed."
    echo "would publish $VERSION from origin/$TARGET_BRANCH @ $TARGET_SHA${PRERELEASE_FLAG:+ (pre-release)}"
    echo "would dispatch a production deploy via ci.yml."
    exit 0
fi

echo "Publishing $VERSION from origin/$TARGET_BRANCH${PRERELEASE_FLAG:+ (pre-release)}"
# shellcheck disable=SC2086
gh release create "$VERSION" \
    $PRERELEASE_FLAG \
    --title "ETHPayServer $VERSION" \
    --notes-file "$NOTES" \
    --target "$TARGET_BRANCH"
