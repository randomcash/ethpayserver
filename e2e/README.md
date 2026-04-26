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
