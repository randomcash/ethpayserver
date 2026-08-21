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
| `E2E_SKIP_AUTH`     | `true` (default-on in remote mode)   | Skips auth tests (passkey origin mismatch) |

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
| `E2E_SKIP_AUTH`     | _(unset; true when E2E_REMOTE is set)_   | Skip auth spec (passkey origin issues remotely)    |

### Running against testnet from a local machine

```bash
cd e2e
npm install
E2E_REMOTE=true E2E_BASE_URL=https://testnet.random.cash npx playwright test
```

Auth tests are skipped by default in remote mode because the WebAuthn
virtual authenticator's RP ID (`localhost`) does not match the remote
domain. Set `E2E_SKIP_AUTH=false` to force-run them once the origin issue
is resolved.

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
