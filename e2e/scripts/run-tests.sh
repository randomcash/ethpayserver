#!/usr/bin/env bash
# Run Playwright with its browser profiles and internal temp files on disk
# instead of a RAM-backed /tmp.
#
# `outputDir` in playwright.config.ts already puts screenshots, traces and
# videos under e2e/test-results, which is disk. What is not covered there is
# everything Playwright itself puts under os.tmpdir() — the chromium profile
# directory in particular — so TMPDIR is what actually needs redirecting.
#
# The trap is the point: an interrupted run (Ctrl-C, a killed job) still
# leaves TMPDIR behind without it, and that is how these accumulate.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

mkdir -p .tmp
scratch="$(mktemp -d .tmp/run-XXXXXX)"
trap 'rm -rf "$scratch"' EXIT

TMPDIR="$(cd "$scratch" && pwd)" npx playwright test "$@"
