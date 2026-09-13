# ethpayserver

A non-custodial EVM payment processor. Merchants receive crypto directly to
addresses derived from their own extended public key; the server never holds a
spending key and cannot move funds.

That guarantee is load-bearing. `validate_xpub` accepts only a base58 **xpub** —
an `xprv` is refused on the version-byte prefix — so a merchant cannot hand over
a spending key even by pasting the wrong line. Anything that would require the
server to hold one is a change to what this product *is*, not a feature.

## This repository is public

So are `payserver-commons` and `payserver-client`. Consequences that are easy to
forget:

- **Never commit a reproduction for an unfixed vulnerability.** Describe the fix
  and the property now enforced; the exploit belongs in the private tracker.
- No session URLs in commits or PR bodies.
- No secrets, obviously — but also no internal hostnames, no private paths.

`central-infrastructure` and `payserver-billing` are private. Deploy config and
billing logic live there and must not migrate here.

## Three repositories, one product

```
payserver-commons     shared types, auth, crypto, rates, ui-kit
       |  pinned BY REVISION in Cargo.toml
       v
ethpayserver          this repo: API, monitor, data-service
       |  pins a published client image in ops/client-image.pin
       v
payserver-client      Leptos/WASM frontend, its own repository
```

**The client never depends on a payserver.** It talks to whichever one is
configured at runtime. Do not add a dependency edge from client to server.

### Changing commons is a three-step dance

`payserver-commons` is pinned by `rev` in the workspace `Cargo.toml`, not by
branch. A change there does not reach this repo until the pin moves.

1. change + merge in commons
2. bump the `rev` here (all crates share one revision), `cargo update -p …`
3. only then does the code see it

To work against a local commons checkout use `scripts/commons.sh link`. **Run
`scripts/commons.sh unlink` before verifying or committing** — a live link builds
against your working copy, so a green build proves nothing about the pin.

## The gate

Exactly what CI runs, and nothing more:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
```

**Do not add `--all-features`.** It surfaces pre-existing errors in `evmmonitor`
that are not in CI's path. Several people have lost an hour to this.

## End-to-end tests

`e2e/` is Playwright against a real server and a real client image.

- **The client is pinned** in `ops/client-image.pin`, and CI tests against that
  pin rather than the deployed client. So the pin can silently fall behind, and
  a feature can ship in the client while every test still runs against a build
  that predates it.
- **When the client renames a class, the e2e selectors and the pin bump must
  move in the same commit.** Split apart, one half looks for controls the other
  half no longer labels that way — and whoever bumps the pin next inherits
  failures they did not cause.
- **Rate limits will fail the suite for the wrong reason.** Defaults are
  `auth_rpm: 5`, `write_rpm: 10`. A full run makes far more than ten writes a
  minute, and the limiter returns 429 **without logging anything** — so the
  server looks healthy while tests fail in no pattern. Run a local server with
  every `RATE_LIMIT_*` at `10000`, as CI does. See `e2e/README.md`.
- Integration tests are `#[ignore]` by convention and need `DATABASE_URL`. CI
  compiles them but does not run them, so run them locally when you touch that
  layer.

## Sensitive paths

Auth, crypto, `evm/`, wallet and key derivation, migrations,
`server/src/api/invoices*`, `server/src/api/payments*`, `server/src/api/payouts*`,
`server/src/api/refunds*`, anything touching movement of funds.

Work in them normally, but say so prominently in the commit message. These are
human-reviewed without exception.

## Conventions that bite

- **`git grep`, not bare `grep`.** `grep` here is `ugrep` and honours
  `.gitignore`, so a plain recursive grep silently skips files. That has produced
  false "clean" results on leak scans.
- **Never `git add -A`.** Check `git status --short | grep '^??'` first and add
  paths explicitly. Build artefacts and symlinked `node_modules` have nearly been
  committed this way.
- **Editing a migration changes its checksum.** `sqlx` stores a SHA-384 of the
  whole file and compares it on startup, so editing a *comment* in an applied
  migration breaks every deploy. Renaming the file is free; changing its bytes is
  not.
- **A test that cannot fail is worse than no test.** Before trusting one, break
  the thing it covers and confirm it goes red. Several tests here have passed for
  the wrong reason — an endpoint that 401s regardless of state, a duplicate-id
  case that fails at the first statement so there is nothing to roll back.

## Deploys

`testnet` deploys on every push, via a dispatch to `central-infrastructure`.
`mainnet` takes release tags only — `vMAJOR.MINOR.PATCH` exactly, no prerelease
suffix — and holds real merchant funds. There is no staging.
