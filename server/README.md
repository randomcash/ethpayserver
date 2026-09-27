# server

Main server for ETHPayServer - unified API and service orchestration.

## Overview

The `server` crate provides the main `ethpayserver` binary that combines all other crates into a single HTTP server with a unified API.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     ETHPayServer Core                       │
├─────────────────────────────────────────────────────────────┤
│  API Layer (axum)                                           │
│  ├── /health, /metrics - Health checks, Prometheus metrics  │
│  ├── /auth       - Passkey, wallet and BIP39 authentication │
│  ├── /stores, /wallets, /invoices, /payments, /evm, /rates  │
│  ├── /users, /admin, /checkout, /dashboard, /plugins        │
│  └── /swagger-ui - API documentation                        │
├─────────────────────────────────────────────────────────────┤
│  Service Layer                                               │
│  ├── EventConsumer, ExpirationSvc, CleanupService            │
│  ├── WatchRetryService, WebhookService                       │
│  └── AuthService - User authentication & sessions            │
├─────────────────────────────────────────────────────────────┤
│  Data Layer                                                  │
│  └── PgDataService - PostgreSQL repositories                 │
└─────────────────────────────────────────────────────────────┘
```

## Running

```bash
# Set required environment variables
export DATABASE_URL="postgres://user:pass@localhost/ethpayserver"

# Optional configuration
export HOST="0.0.0.0"      # Default: 127.0.0.1
export PORT="3000"         # Default: 3000
export LOG_LEVEL="info"    # Default: info
export ENABLE_SWAGGER="true"  # Default: true

# Run the server
cargo run --release --bin ethpayserver
```

## API Endpoints

A representative sample, not the full surface — stores alone has grown wallet,
settings, webhook, payment-methods, token-policy and payouts sub-resources not
listed here, and whole groups (`/wallets`, `/payments`, `/rates`,
`/dashboard`, `/users`, `/admin`, `/checkout`, `/plugins`) are omitted
entirely. See the root [README's API Endpoints section](../README.md#api-endpoints)
for a fuller (still non-exhaustive) list, `/swagger-ui` on a running instance
for the generated and current one, or `server/src/api/mod.rs` for the router.

### Health

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Full health check with database status |
| `GET /health/live` | Liveness probe (always 200 if running) |
| `GET /health/ready` | Readiness probe (checks database) |

### Stores (mounted at `/stores`)

| Endpoint | Description |
|----------|-------------|
| `GET /stores` | List stores for authenticated user |
| `POST /stores` | Create a new store |
| `GET /stores/{id}` | Get store details |
| `PUT /stores/{id}` | Update store |
| `DELETE /stores/{id}` | Archive store |
| `GET /stores/{id}/members` | List store members |
| `POST /stores/{id}/members` | Add member to store |
| `PUT /stores/{id}/members/{user_id}` | Update member role |
| `DELETE /stores/{id}/members/{user_id}` | Remove member |

### Invoices (mounted at `/invoices`)

| Endpoint | Description |
|----------|-------------|
| `GET /invoices` | List invoices with filters |
| `POST /invoices` | Create a new invoice |
| `GET /invoices/{id}` | Get invoice details |
| `POST /invoices/{id}/cancel` | Cancel a pending invoice |
| `POST /invoices/{id}/refund` | Always refuses (501) — refunds are the merchant's job; this server holds no spending key in the shipped build (see `evm/README.md`'s `hot-wallet` feature entry) |

Invoice expiration is not an endpoint — the background `ExpirationSvc`
(see the root README's Architecture diagram) expires pending invoices on its
own schedule.

### Auth (mounted at `/auth`)

| Endpoint | Description |
|----------|-------------|
| `POST /auth/passkey/new-user/start` | Start passkey registration for new user |
| `POST /auth/passkey/new-user/complete` | Complete passkey registration |
| `POST /auth/passkey/login/start` | Start passkey login |
| `POST /auth/passkey/login/complete` | Complete passkey login |
| `POST /auth/wallet/new-user/start` | Start wallet registration for new user |
| `POST /auth/wallet/new-user/complete` | Complete wallet registration |
| `POST /auth/wallet/login/start` | Start wallet login |
| `POST /auth/wallet/login/complete` | Complete wallet login |
| `POST /auth/recovery/start` | Start account recovery |
| `POST /auth/recovery/complete` | Complete account recovery |
| `GET /auth/devices` | List user's devices |
| `DELETE /auth/devices/{id}` | Revoke a device |
| `POST /auth/logout` | Logout current session |
| `POST /auth/logout/all` | Logout all sessions |

### EVM (mounted at `/evm`)

| Endpoint | Description |
|----------|-------------|
| `GET /evm/networks` | List supported networks |
| `GET /evm/networks/{id}` | Get network details |
| `GET /evm/tokens` | List tokens (admin) |
| `POST /evm/tokens` | Create token (admin) |
| `GET /evm/tokens/{id}` | Get token (admin) |
| `PUT /evm/tokens/{id}` | Update token (admin) |
| `DELETE /evm/tokens/{id}` | Delete token (admin) |

## Swagger UI

When `ENABLE_SWAGGER=true`, the Swagger UI is available at `/swagger-ui`.

## Dependencies

- `auth` - Authentication service
- `data-service` - PostgreSQL data access
- `evm` - EVM blockchain operations
- `types`, `api-types` - Common types and REST request/response shapes
- `crypto`, `rates` - Cryptographic primitives, fiat/crypto exchange rates
- `payserver-plugin-api`, `payserver-plugin-host` - Plugin manifests and the
  wasmtime host that runs them (see the root README's [Plugins](../README.md#plugins))
