#!/usr/bin/env bash
# Refuses to let a build proceed with an empty SENTRY_RELEASE.
#
# `option_env!("SENTRY_RELEASE")` reads this at compile time and resolves to
# a silent `None` if it's ever empty - a rename, a typo, or a step reordering
# that clobbers it all compile clean and look identical to success. Called
# from the CI steps that build the binaries, right after truncating
# SENTRY_RELEASE to match build_sha's length.
#
# Usage: check-sentry-release.sh "$SENTRY_RELEASE"
set -euo pipefail

if [ -z "${1:-}" ]; then
  echo "::error::SENTRY_RELEASE is empty; refusing to build a binary with no Sentry release" >&2
  exit 1
fi
