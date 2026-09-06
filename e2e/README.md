# E2E Tests

Playwright end-to-end tests for ethpayserver.

## Local mode (default)

Runs against a local backend and trunk dev server. Requires PostgreSQL with an
`ethpayserver_e2e` database.

```bash
cd e2e
npm install
npx playwright test
```

The config spawns `cargo run --release --bin ethpayserver` and `trunk serve`
automatically (skipped if already running via `reuseExistingServer`).

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

Auth tests run in remote mode. They used to be skipped by default, on the
grounds that the virtual authenticator's RP ID (`localhost`) could not match
the remote domain — that explanation was wrong. `WebAuthn.addVirtualAuthenticator`
has no RP ID parameter; the RP ID comes from the page's origin when
`navigator.credentials.create()` runs. The deployed server logs
`rp_id=testnet.random.cash rp_origin=https://testnet.random.cash` at startup,
so there is nothing to mismatch.

What actually made the spec unusable remotely was its `resetDatabase()` call,
now removed — every test there creates its own uniquely-named account and never
needed an empty database.

`E2E_SKIP_AUTH=true` still skips them explicitly.

**Still local-only:** `invoices`, `stores`, `payment-methods`, `ui-interactions`
and `webhooks` all call `resetDatabase()`. Never point those at a shared
environment — `E2E_DATABASE_URL` defaults to localhost, so they fail closed
rather than deleting live data, but that is luck rather than design.

Note separately that `scout.spec.ts` was seen failing to establish a session
after passkey registration against testnet (#56). That is a real, open gap and
unrelated to the RP ID story above.

## Test wallet maintenance (`scripts/`)

Both scripts are operator tools — nothing in CI runs them.

```bash
# Mint a throwaway Sepolia wallet: prints the phrase, the spender address to
# fund, and the merchant xpub. Store the phrase as the E2E_TEST_MNEMONIC secret.
node scripts/new-test-wallet.mjs

# Reclaim funds parked in derived receive addresses (RCS-202). Dry run by
# default; pass --execute to broadcast.
E2E_TEST_MNEMONIC="..." E2E_SEPOLIA_RPC_URL="https://..." \
  node scripts/sweep-test-wallet.mjs --scan 50
```

Each nightly run moves `INVOICE_AMOUNT_ETH` (0.0001) from the spender to an
address derived from the *same* seed, so the principal is parked rather than
spent — only gas (~0.00002/run at 0.94 gwei) is actually consumed. At 0.05
funded that is ~416 runs without sweeping, ~2,500 with. The spec emits a
`::warning::` once fewer than 20 runs' worth remain.

## Synthetic payment (`tests/synthetic-payment.spec.ts`)

The only test that exercises the money path for real: it creates an invoice over
the API, broadcasts an actual Sepolia transaction to the address the server
derived, waits for `paid` on the public checkout WebSocket, and asserts the store
webhook fired with a valid HMAC signature.

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
