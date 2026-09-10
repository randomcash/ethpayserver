# E2E Tests

Playwright end-to-end tests for ethpayserver.

## Local mode (default)

Runs against a local backend and trunk dev server. Requires PostgreSQL with an
`ethpayserver_e2e` database, and a checkout of
[payserver-client](https://github.com/randomcash/payserver-client) — the
frontend is its own repository now. A sibling directory is assumed; set
`PAYSERVER_CLIENT_DIR` if yours is elsewhere.

```bash
cd e2e
npm install
npx playwright test
```

The config spawns `cargo run --release --bin ethpayserver` and `trunk serve`
automatically (skipped if already running via `reuseExistingServer`). CI does
not use either: it runs the server binary it just built and the published
payserver-client image pinned in `ops/client-image.pin`, so the suite exercises
the real nginx routing rather than the dev server's proxy.

## What is not here

Layout regression tests moved to
[payserver-client](https://github.com/randomcash/payserver-client) with the
frontend. They inject `styles.css` into a blank page and assert computed style —
no server, no auth — so they belong with the stylesheet they test, and a CSS
specificity regression now fails the repository that owns the CSS instead of
this one.

## Remote mode

Runs the same suite against a deployed environment (e.g. testnet) without
spawning local servers or touching the database.

```bash
E2E_REMOTE=true npx playwright test
```

This sets sane defaults:

| Variable            | Default when `E2E_REMOTE` is set     | Purpose                                    |
|---------------------|--------------------------------------|--------------------------------------------|
| `E2E_BASE_URL`      | `https://testnet.random.cash`        | Frontend URL for Playwright `baseURL`      |
| `E2E_API_URL`       | `https://testnet.random.cash`        | API base URL                               |
| `E2E_SKIP_DB_RESET` | `true` (implicit in remote mode)     | Skips `TRUNCATE` in `fixtures/db.ts`       |
| `E2E_SKIP_AUTH`     | _(unset)_                            | Set `true` to skip the auth spec           |

All defaults can be overridden explicitly:

```bash
E2E_REMOTE=true \
E2E_BASE_URL=https://staging.random.cash \
E2E_SKIP_AUTH=false \
  npx playwright test
```

### Environment variables reference

| Variable            | Default (local)                          | Description                                        |
|---------------------|------------------------------------------|----------------------------------------------------|
| `E2E_REMOTE`        | _(unset)_                                | Enable remote mode (skip webServer, adjust defaults)|
| `E2E_BASE_URL`      | `http://localhost:8080`                  | Frontend base URL                                  |
| `E2E_API_URL`       | `http://localhost:3000`                  | API base URL                                       |
| `E2E_DATABASE_URL`  | `postgres://postgres:postgres@localhost:5432/ethpayserver_e2e` | Database connection string      |
| `E2E_SKIP_DB_RESET` | _(unset)_                                | Skip database truncate-and-seed in `beforeAll`     |
| `E2E_SKIP_AUTH`     | _(unset)_                                | Set `true` to skip the auth spec                   |

### Running against testnet from a local machine

```bash
cd e2e
npm install
E2E_REMOTE=true E2E_BASE_URL=https://testnet.random.cash npx playwright test
```

Auth tests are skipped in remote mode, and the reason has been wrong twice.

It is **not** the RP ID. The old note claimed the virtual authenticator's RP ID
(`localhost`) could not match a remote domain, but `WebAuthn.addVirtualAuthenticator`
has no RP ID parameter — it comes from the page's origin when
`navigator.credentials.create()` runs. The deployed server logs
`rp_id=testnet.random.cash rp_origin=https://testnet.random.cash`, and a single
registration against live testnet completes end to end.

It is **not** `resetDatabase()`. `fixtures/db.ts` returns early when `E2E_REMOTE`
is `true`, so it is already a no-op remotely.

The real blocker is **rate limiting**: the auth tier allows 5 requests per minute
per IP (`RATE_LIMIT_AUTH`), and this spec performs five registrations plus a login
well inside a minute. Remotely it returns `HTTP 429: Too many requests` and three
of five tests fail. `scout.spec.ts` registers once, which is why it passes
remotely and this does not.

`E2E_SKIP_AUTH=false` force-runs them; expect 429s until either the spec paces
itself under the limit or test traffic gets a higher one.

**Still local-only for a different reason:** `invoices`, `stores`,
`payment-methods`, `ui-interactions` and `webhooks` all call `resetDatabase()`.
The guard that protects a shared database is `E2E_REMOTE=true` in
`fixtures/db.ts` — *not* the `E2E_DATABASE_URL` localhost default. So with
`E2E_REMOTE` unset and `E2E_DATABASE_URL` pointed at a shared database, those
specs will truncate it.

Note separately that `scout.spec.ts` was seen failing to establish a session
after passkey registration against testnet (#56). That is a real, open gap and
unrelated to the RP ID story above.

## Test wallet maintenance (`scripts/`)

These are operator tools — nothing in CI runs them.

```bash
# Mint a throwaway Sepolia wallet: prints the phrase, the spender address to
# fund, and the merchant xpub. Store the phrase as the E2E_TEST_MNEMONIC secret.
node scripts/new-test-wallet.mjs

# Reclaim funds parked in derived receive addresses. Dry run by
# default; pass --execute to broadcast.
E2E_TEST_MNEMONIC="..." E2E_SEPOLIA_RPC_URL="https://..." \
  node scripts/sweep-test-wallet.mjs --scan 1000
```

`--scan` has to cover the *whole* history, not a window near zero. The
derivation counter used to live on the payment method, so a fresh store
each night restarted at 0 and the parked funds piled up on the first few
addresses; the counter now lives on an account-level wallet keyed by the xpub,
so the indices march outwards three per night and never restart. A scan that
stops short reports nothing to sweep rather than failing, so the default is 1000
(over three years of nightlies). Raise it rather than trim it.

Each nightly run makes three payments of a random 0.00005–0.00015 ETH
(~0.0003/run on average) from the spender to addresses derived from the *same*
seed, so the principal is parked rather than spent — only gas (~0.00006/run at
0.94 gwei, three transfers) is actually consumed. At 0.05 funded that is ~140
runs without sweeping, ~800 with.

The spec emits a `::warning::` once fewer than 20 runs' worth remain, and sizes
a run at its **worst case** — three maximum draws plus a gas reserve each, the
larger of a 0.0005 floor and three times the live gas price, so ~0.00195 while
Sepolia is quiet — because the amounts of future runs have not been drawn yet
and the gas price they will pay is not today's. That is a
deliberately pessimistic ~0.039 ETH line (it assumes the parked principal is
gone), so a wallet funded at 0.05 and never swept starts warning after a few
weeks. Sweep it, or fund ~0.1.

## Leftover synthetic-payment stores (`scripts/sweep-e2e-stores.mjs`)

The synthetic-payment spec creates a store per run and now removes it again in
an `afterEach`. This script clears the ones that accumulated before
that landed, and anything a run abandoned by dying outright.

```bash
E2E_API_URL=https://testnet.random.cash E2E_REMOTE=true E2E_API_TOKEN=ak_... \
  node scripts/sweep-e2e-stores.mjs          # lists only
E2E_API_URL=... E2E_REMOTE=true E2E_API_TOKEN=ak_... \
  node scripts/sweep-e2e-stores.mjs --execute
```

It only ever touches names matching the exact stamp the spec generates
(`e2e-synthetic-2026-08-27T17-29-33-596Z`), and `GET /stores` only returns the
token's own stores, so it cannot reach another account. Names that start with
`e2e-synthetic-` but do not match the full shape are listed and left alone.

Note that `DELETE /stores/{id}` **archives** — it is
`UPDATE stores SET archived = true`, not a row delete. The store leaves the UI's
store list (archived is hidden behind a checkbox) but `GET /stores` still
returns it, its payment method, webhook and invoices all remain, and a second
sweep reports it as already archived rather than deleting it again. Removing the
rows themselves needs database access.

## Synthetic payment (`tests/synthetic-payment.spec.ts`)

The only test that exercises the money path for real: it creates invoices over
the API, broadcasts actual Sepolia transactions to the addresses the server
derived, waits for `paid` on the public checkout WebSocket, and asserts the store
webhook fired with a valid HMAC signature.

Three invoices per run, on one store and one payment method, paid one at a time.
That is the regression test for address reuse: the addresses come from a single xpub
and a single counter, and when the counter was wrong two invoices were quoted the
same address — which one invoice per run can never see. The run asserts the three
addresses are distinct, that the counter on the wallet the store derives from
advanced by exactly three, that the payment method agrees with that
wallet rather than keeping a number of its own, and that each payment paid its
own invoice and no other.

The three payments share an 18-minute wall clock, checked against the job's
30-minute `timeout-minutes` in `.github/workflows/e2e-scheduled.yml`: each
payment keeps its full per-payment timeouts, but a run that would overrun the job
fails inside Playwright — naming the invoice it was waiting on — rather than
being killed at the cap with no report and a leaked store.

The pure parts (the amount draw, the shared budget) live in
`fixtures/synthetic-payment.ts` and are unit-tested by
`tests/synthetic-payment-helpers.spec.ts`, which runs in every suite: the spec
itself only runs on a runner holding secrets, so arithmetic left inside it is
unverified until a nightly spends real ETH to find out.

It is **off unless `E2E_SYNTHETIC_PAYMENT=true`**, because it spends testnet ETH
and needs secrets — the in-pipeline `e2e` job must not pick it up. When it *is*
on, missing configuration fails the test rather than skipping it: a silently
skipped money path is the gap this test exists to close.

```bash
E2E_REMOTE=true E2E_SYNTHETIC_PAYMENT=true \
E2E_TEST_MNEMONIC="..." E2E_API_TOKEN=ak_... E2E_SEPOLIA_RPC_URL=https://... \
  npx playwright test tests/synthetic-payment.spec.ts
```

| Variable                   | Required | Description                                                        |
|----------------------------|----------|--------------------------------------------------------------------|
| `E2E_SYNTHETIC_PAYMENT`    | yes      | `true` to run the spec at all                                      |
| `E2E_TEST_MNEMONIC`        | yes      | BIP39 phrase — merchant xpub **and** the spending wallet           |
| `E2E_API_TOKEN`            | yes      | API key (`ak_...`) allowed to create stores and invoices           |
| `E2E_SEPOLIA_RPC_URL`      | yes      | Sepolia RPC endpoint used to broadcast                             |
| `E2E_WEBHOOK_PUBLIC_URL`   | no       | Skip the cloudflared quick tunnel and use this base URL instead    |
| `E2E_WEBHOOK_PORT`         | no       | Bind the sink to a fixed port (pairs with the above)               |
| `E2E_CLOUDFLARED_BIN`      | no       | Path to `cloudflared` (default: on `PATH`)                         |
| `E2E_API_PREFIX`           | no       | API path prefix (default `/api` remote, empty locally)             |

### One-time setup

1. Generate a BIP39 mnemonic **for testnet only** and store it as the repo secret
   `E2E_TEST_MNEMONIC`. Never commit it, and never reuse a mainnet phrase.
2. Fund the spending wallet at `m/44'/60'/9'/0/0` from a Sepolia faucet. The test
   prints the address and its balance on every run, and fails with that address
   when the balance hits zero.
3. Create a user on testnet, mint an API key, store it as `E2E_API_TOKEN`.
4. Store a Sepolia RPC endpoint as `E2E_SEPOLIA_RPC_URL`.
5. Optionally set `HEALTHCHECK_E2E_URL` to a healthchecks.io check so a failure —
   or a run that never happens — pages a human without anyone opening Actions.

Nothing else needs provisioning: the test creates its own store, payment method
and webhook config on each run.

### Why the funds are not burned

The store's xpub is `m/44'/60'/0'` of the same mnemonic, and the server derives
payment addresses at `0/{index}` beneath it (`evm/src/wallet.rs`). Every address
it pays into is therefore spendable from `E2E_TEST_MNEMONIC` at
`m/44'/60'/0'/0/{index}` — the faucet ETH can be swept back to the spender rather
than being stranded. The spender sits at account index 9 so it can never collide
with a receive address.

### Deliberate non-cleanup

Each run creates a fresh store (`e2e-synthetic-<timestamp>`) and leaves it
behind. The derivation index advances per payment method, so reusing one store
would couple each run to the last; and on a failure the invoice and its payment
rows are the evidence. Prune them by hand if testnet gets noisy.
