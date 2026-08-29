# ETHPayServer

A self-hosted Ethereum and EVM-networks payment processor built in Rust.

Accept ETH and ERC20 tokens (USDC, USDT, DAI, WBTC, etc.) across 50+ EVM-compatible chains with a single codebase.

## Overview

ETHPayServer is a free, open-source payment processor that enables merchants to accept cryptocurrency payments on Ethereum and all EVM-compatible chains (Polygon, Arbitrum, Optimism, Base, BSC, and more).

### Key Features

- **Multi-Chain Support** - Works across 12+ EVM mainnets and 8+ testnets
- **Native + Token Payments** - Accept ETH and whitelisted ERC20 tokens
- **Payment Monitoring** - Real-time detection via WebSocket subscriptions
- **Reorg Protection** - Handles blockchain reorganizations safely
- **Horizontal Scaling** - Separate monitor binary with Redis event bridge
- **Direct RPC Support** - Connect to your own nodes or use providers (Alchemy, Infura)
- **Testnet Support** - Full testnet support for development and testing

## Quick Start (Local Development)

### Prerequisites

- Rust (pinned via `rust-toolchain.toml`)
- PostgreSQL 14+
- Redis 7+
- Ethereum RPC access (Alchemy, Infura, or self-hosted node)

### 1. Clone and Setup

```bash
# Clone the repository
git clone git@gitlab.com:random.cash/ethpayserver.git
cd ethpayserver
```

### 2. Start Services

```bash
# Start PostgreSQL and Redis with docker-compose
docker compose -f docker-compose.local.yml up -d

# Verify services are running
docker compose -f docker-compose.local.yml ps
```

### 3. Database Setup

```bash
# Install sqlx-cli
cargo install sqlx-cli --no-default-features --features postgres

# Run migrations (database is auto-created by docker-compose)
DATABASE_URL="postgres://postgres:postgres@localhost/ethpayserver" \
sqlx migrate run --source data-service/migrations/postgres
```

### 4. Run the API Server

```bash
# Set environment variables
export DATABASE_URL="postgres://postgres:postgres@localhost/ethpayserver"
export HOST="127.0.0.1"
export PORT="3000"
export RUST_LOG="info"
export ENABLE_SWAGGER="true"

# Run the server
cargo run --release --bin ethpayserver
```

The API server will be available at `http://localhost:3000` with Swagger UI at `/swagger-ui`.

### 5. Run the Payment Monitor

The payment monitor is a separate binary that watches blockchain activity and publishes events to Redis.

```bash
# Build the monitor
cargo build --release --bin evmmonitor --features monitor-bin

# Run on mainnet (example with Ethereum)
EVMMONITOR_REDIS_URL=redis://localhost:6379 \
EVMMONITOR_CHAINS=1 \
EVMMONITOR_CHAIN_1_RPC_HTTP=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
EVMMONITOR_CHAIN_1_RPC_WS=wss://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
RUST_LOG=info \
./target/release/evmmonitor
```

#### Running on Testnet (Sepolia)

```bash
EVMMONITOR_REDIS_URL=redis://localhost:6379 \
EVMMONITOR_CHAINS=11155111 \
EVMMONITOR_CHAIN_11155111_RPC_HTTP=https://sepolia.infura.io/v3/YOUR_KEY \
EVMMONITOR_CHAIN_11155111_RPC_WS=wss://sepolia.infura.io/ws/v3/YOUR_KEY \
RUST_LOG=info \
./target/release/evmmonitor
```

#### Multi-Chain Configuration

```bash
# Monitor multiple chains simultaneously
EVMMONITOR_REDIS_URL=redis://localhost:6379 \
EVMMONITOR_CHAINS=1,137,42161 \
EVMMONITOR_CHAIN_1_RPC_HTTP=https://eth-mainnet.g.alchemy.com/v2/KEY \
EVMMONITOR_CHAIN_1_RPC_WS=wss://eth-mainnet.g.alchemy.com/v2/KEY \
EVMMONITOR_CHAIN_137_RPC_HTTP=https://polygon-mainnet.g.alchemy.com/v2/KEY \
EVMMONITOR_CHAIN_137_RPC_WS=wss://polygon-mainnet.g.alchemy.com/v2/KEY \
EVMMONITOR_CHAIN_42161_RPC_HTTP=https://arb-mainnet.g.alchemy.com/v2/KEY \
EVMMONITOR_CHAIN_42161_RPC_WS=wss://arb-mainnet.g.alchemy.com/v2/KEY \
./target/release/evmmonitor
```

Or use a config file (`evmmonitor.toml`):

```toml
[bridge]
redis_url = "redis://localhost:6379"

[[chains]]
chain_id = 1
rpc_http = "https://eth-mainnet.g.alchemy.com/v2/KEY"
rpc_ws = "wss://eth-mainnet.g.alchemy.com/v2/KEY"

[[chains]]
chain_id = 137
rpc_http = "https://polygon-mainnet.g.alchemy.com/v2/KEY"
rpc_ws = "wss://polygon-mainnet.g.alchemy.com/v2/KEY"
```

```bash
./target/release/evmmonitor --config evmmonitor.toml
```

## Supported Networks

### Mainnets

| Network | Chain ID | Native Token |
|---------|----------|--------------|
| Ethereum | 1 | ETH |
| Polygon | 137 | POL |
| Arbitrum | 42161 | ETH |
| Optimism | 10 | ETH |
| Base | 8453 | ETH |
| Avalanche | 43114 | AVAX |
| BNB Smart Chain | 56 | BNB |
| zkSync Era | 324 | ETH |
| Linea | 59144 | ETH |
| Scroll | 534352 | ETH |
| Fantom | 250 | FTM |
| Gnosis | 100 | xDAI |

### Testnets

| Network | Chain ID | Native Token |
|---------|----------|--------------|
| Sepolia | 11155111 | ETH |
| Holesky | 17000 | ETH |
| Polygon Amoy | 80002 | POL |
| Arbitrum Sepolia | 421614 | ETH |
| Optimism Sepolia | 11155420 | ETH |
| Base Sepolia | 84532 | ETH |
| Avalanche Fuji | 43113 | AVAX |
| BSC Testnet | 97 | BNB |

### Supported Tokens

- Native tokens (ETH, MATIC, AVAX, BNB, etc.)
- USDC, USDT, DAI, WBTC
- Custom whitelisted ERC20 tokens

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     ETHPayServer                             │
├─────────────────────────────────────────────────────────────┤
│  API Server (ethpayserver binary)                            │
│  ├── /health     - Health checks                             │
│  ├── /stores     - Store management                          │
│  ├── /invoices   - Invoice CRUD                              │
│  ├── /auth       - Authentication (passkey, wallet, BIP39)   │
│  ├── /evm        - EVM operations (tokens, networks)         │
│  └── /swagger-ui - API documentation                         │
├─────────────────────────────────────────────────────────────┤
│  Background Services                                         │
│  ├── EventConsumer   - Processes events from Redis           │
│  ├── ExpirationSvc   - Expires old invoices                  │
│  ├── CleanupService  - Unwatches completed addresses         │
│  ├── WatchRetryService - Retries failed watch commands       │
│  └── WebhookService  - Delivers webhook notifications        │
├─────────────────────────────────────────────────────────────┤
│  Data Layer (PostgreSQL)                                     │
│  └── PgDataService - Repository implementations              │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ Redis pub/sub
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  evmmonitor (separate binary - can run multiple instances)   │
│  ├── WebSocket subscriptions for real-time blocks            │
│  ├── ERC20 Transfer event detection                          │
│  ├── Native ETH transfer detection                           │
│  ├── Confirmation tracking                                   │
│  └── Reorg detection                                         │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ RPC (HTTP + WebSocket)
                              ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│  Ethereum RPC   │  │  Polygon RPC    │  │  Arbitrum RPC   │
└─────────────────┘  └─────────────────┘  └─────────────────┘
```

## Project Structure

```
ethpayserver/
├── server/            # Main API server (ethpayserver binary)
├── evm/               # EVM blockchain interaction (evmmonitor binary)
├── data-service/      # PostgreSQL + Redis data access layer
└── memos/             # Project documentation and notes
```

### Crates

| Crate | Binary | Description | README |
|-------|--------|-------------|--------|
| `server` | `ethpayserver` | Main API server with REST endpoints, Swagger UI, background services | [server/README.md](./server/README.md) |
| `evm` | `evmmonitor` | Chain abstraction, payment monitoring, HD wallet derivation | [evm/README.md](./evm/README.md) |
| `data-service` | - | PostgreSQL repositories, Redis persistence | [data-service/README.md](./data-service/README.md) |

### External Dependencies (payserver-commons)

Shared libraries from [payserver-commons](https://gitlab.com/random.cash/payserver-commons):

| Crate | Description | README |
|-------|-------------|--------|
| `types` | Common types: `Network`, `InvoiceData`, `PaymentData`, repository traits | [types/README.md](https://gitlab.com/random.cash/payserver-commons/-/blob/main/types/README.md) |
| `auth` | User authentication: passkeys, Ethereum wallets, BIP39 recovery | [auth/README.md](https://gitlab.com/random.cash/payserver-commons/-/blob/main/auth/README.md) |
| `crypto` | Cryptographic primitives: Argon2id, AES-256, X25519, Ed25519 | [crypto/README.md](https://gitlab.com/random.cash/payserver-commons/-/blob/main/crypto/README.md) |

## API Endpoints

### Health

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Full health check with database status |
| GET | `/health/live` | Liveness probe |
| GET | `/health/ready` | Readiness probe (checks database) |

### Authentication

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/auth/passkey/new-user/start` | Start passkey registration |
| POST | `/auth/passkey/new-user/complete` | Complete passkey registration |
| POST | `/auth/passkey/login/start` | Start passkey login |
| POST | `/auth/passkey/login/complete` | Complete passkey login |
| POST | `/auth/wallet/new-user/start` | Start wallet registration |
| POST | `/auth/wallet/new-user/complete` | Complete wallet registration |
| POST | `/auth/wallet/login/start` | Start wallet login |
| POST | `/auth/wallet/login/complete` | Complete wallet login |
| POST | `/auth/recovery/start` | Start BIP39 recovery |
| POST | `/auth/recovery/complete` | Complete recovery |
| GET | `/auth/devices` | List user's devices |
| DELETE | `/auth/devices/{id}` | Revoke a device |
| POST | `/auth/logout` | Logout current session |

### Stores

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/stores` | List stores for authenticated user |
| POST | `/stores` | Create a new store |
| GET | `/stores/{id}` | Get store details |
| PUT | `/stores/{id}` | Update store |
| DELETE | `/stores/{id}` | Archive store |

### Invoices

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/invoices` | List invoices with filters |
| POST | `/invoices` | Create a new invoice |
| GET | `/invoices/{id}` | Get invoice details |
| POST | `/invoices/{id}/cancel` | Cancel a pending invoice |

### EVM

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/evm/networks` | List supported networks |
| GET | `/evm/networks/{id}` | Get network details |
| GET | `/evm/tokens` | List tokens (admin) |
| POST | `/evm/tokens` | Create token (admin) |
| PUT | `/evm/tokens/{id}` | Update token (admin) |
| DELETE | `/evm/tokens/{id}` | Delete token (admin) |

## Redis Communication

The API server and evmmonitor communicate via Redis pub/sub:

### Channels

| Channel | Direction | Description |
|---------|-----------|-------------|
| `evmmonitor:commands` | API -> Monitor | Commands to watch/unwatch addresses |
| `evmmonitor:events` | Monitor -> API | Payment events, confirmations, reorgs |

### Commands (API Server -> Monitor)

```json
{"type": "watch_address", "chain_id": 1, "address": "0x...", "invoice_id": "uuid"}
{"type": "unwatch_address", "chain_id": 1, "address": "0x..."}
{"type": "get_status"}
```

### Events (Monitor -> API Server)

- `PaymentDetected` - Payment received (unconfirmed)
- `PaymentConfirmed` - Payment reached required confirmations
- `ReorgDetected` - Chain reorganization detected
- `AddressWatched` - Address monitoring started
- `AddressUnwatched` - Address removed from watch list

### Redis Keys

| Pattern | Description |
|---------|-------------|
| `evmwatch:addr:{chain_id}:{address}` | Watched address -> invoice_id mapping |

## Database Schema

### Migrations

Located in `data-service/migrations/postgres/`:

```bash
# Run migrations
sqlx migrate run --source data-service/migrations/postgres

# Revert last migration
sqlx migrate revert --source data-service/migrations/postgres
```

### Tables

**Auth Tables:**
- `users` - User accounts
- `sessions` - Active sessions
- `devices` - Registered devices/passkeys
- `wallets` - Linked Ethereum wallets

**Store Tables:**
- `stores` - Merchant stores
- `store_roles` - Role definitions (Owner, Manager, Employee, Guest)
- `user_stores` - User-store membership
- `store_wallets` - Store xpub keys for address derivation
- `store_webhooks` - Webhook configuration

**Payment Tables:**
- `invoices` - Payment invoices
- `payments` - Detected payments
- `payment_events` - Audit log
- `watched_addresses` - PostgreSQL persistence for watched addresses
- `tokens` - Configured ERC20 tokens

## Development Status

### What's Implemented

#### Infrastructure
- [x] Workspace structure with modular crates
- [x] PostgreSQL database with migrations
- [x] Repository pattern (Reader/Writer traits)
- [x] Redis event bridge for horizontal scaling
- [x] Unified API server with Swagger UI
- [x] Shared libraries in `payserver-commons`

#### Authentication
- [x] Passkey/WebAuthn authentication
- [x] Ethereum wallet authentication (EIP-191)
- [x] BIP39 mnemonic recovery
- [x] Session and device management
- [x] Multi-tenant stores with role-based permissions

#### EVM Support
- [x] 12 mainnet networks configured
- [x] 8 testnet networks configured (Sepolia, Holesky, etc.)
- [x] HD wallet derivation (BIP-32/44)
- [x] RPC provider abstraction (Alloy)
- [x] Token management API

#### Payment Monitoring
- [x] Standalone evmmonitor binary
- [x] WebSocket block subscriptions (real-time)
- [x] HTTP polling fallback
- [x] Native ETH and ERC20 transfer detection
- [x] Confirmation tracking
- [x] Reorg detection
- [x] Bidirectional Redis commands

#### Payment Flow
- [x] Invoice creation with address derivation
- [x] Event consumer for payment events
- [x] Invoice status updates (pending -> processing -> paid)
- [x] Invoice expiration service
- [x] Address cleanup service
- [x] Watch retry service
- [x] Webhook notifications with HMAC-SHA256

#### Exchange Rates
- [x] Kraken + CoinGecko rate providers with caching and fallback
- [x] Fiat-to-crypto conversion integrated into invoice creation
- [x] Rate staleness validation

#### Real-time Updates
- [x] Payment status WebSocket endpoint (/ws authenticated, /checkout/ws public)
- [x] tokio broadcast for event fan-out

#### Observability
- [x] Prometheus metrics endpoint (/metrics)
- [x] HTTP request counters and latency histograms
- [x] Payment, webhook, invoice, and DB pool metrics

#### Load Testing
- [x] Goose-based load test scenarios (invoice create, list, webhook burst)
- [x] WebSocket connection stress test
- [x] Baseline targets documented

### What's Remaining

- [ ] Load test: first live run, pending `LOADTEST_API_KEY` / `LOADTEST_STORE_ID` secrets
      (CI job and automated regression comparison already exist)
- [ ] Security audit (pre-mainnet)
- [ ] Public /rates API endpoint for frontends

## Testing

```bash
# Run all tests
cargo test

# Run with test utilities
cargo test --features test-utils

# Run specific crate tests
cargo test -p evm
cargo test -p server
cargo test -p data-service
```

## Docker

Dockerfiles are located in `/docker`:

| File | Description |
|------|-------------|
| `ethpayserver.Dockerfile` | API server (ethpayserver) |
| `evmmonitor.Dockerfile` | Chain monitor (evmmonitor) |
| `docker-compose.local.yml` | Local development (default credentials, exposed ports) |
| `docker-compose.prod.yml` | Production (isolated networks, required secrets) |
| `.env.example` | Environment variable template |

### Build Individual Images

Build from ethpayserver directory:

```bash
cd ./ethpayserver

# Build API server
docker build -f docker/ethpayserver.Dockerfile -t ethpayserver .

# Build chain monitor
docker build -f docker/evmmonitor.Dockerfile -t evmmonitor .
```

### Run Local Development Stack

```bash
cd ./ethpayserver

# Copy and configure environment
cp docker/.env.example docker/.env
# Edit .env with your RPC URLs

# Start all services (postgres, redis, server, evmmonitor)
docker compose -f docker/docker-compose.local.yml up --build

# Or run in background
docker compose -f docker/docker-compose.local.yml up -d --build
```

### Run Production Stack

```bash
cd ./ethpayserver

# Copy and configure environment
cp docker/.env.example docker/.env
# Edit .env with production credentials and RPC URLs

# Start all services
docker compose -f docker/docker-compose.prod.yml up --build -d

# View logs
docker compose -f docker/docker-compose.prod.yml logs -f
```

Production differences from local:
- Requires `POSTGRES_PASSWORD` and `EVMMONITOR_CHAINS` (will fail without)
- Swagger UI disabled by default
- Internal services (postgres, redis) not exposed to host
- Isolated networks (internal for services, external for API only)
- Redis persistence enabled (`appendonly yes`)

### Run Individual Containers

```bash
# API server
docker run -p 3000:3000 \
  -e DATABASE_URL=postgres://user:pass@host/ethpayserver \
  -e REDIS_URL=redis://host:6379 \
  ethpayserver

# Chain monitor
docker run \
  -e EVMMONITOR_REDIS_URL=redis://host:6379 \
  -e EVMMONITOR_CHAINS=1,137 \
  -e EVMMONITOR_CHAIN_1_RPC_HTTP=https://eth-mainnet.g.alchemy.com/v2/KEY \
  -e EVMMONITOR_CHAIN_1_RPC_WS=wss://eth-mainnet.g.alchemy.com/v2/KEY \
  evmmonitor
```

## Security

ETHPayServer implements several security measures:

- Address validation (checksum verification)
- Whitelisted token contracts only
- Confirmation requirements per chain
- Reorg detection and handling
- Webhook HMAC-SHA256 signatures
- Audit logging for all payment events

## Performance Targets

- Invoice creation: <100ms
- Payment detection: <5 seconds after confirmation
- Concurrent monitored addresses: 10,000+
- Uptime: 99.9%

## Reproducible Builds

ETHPayServer uses [Nix flakes](https://nix.dev/concepts/flakes) with
[crane](https://github.com/ipetkov/crane) to produce bit-identical binaries
from a given commit. This lets merchants, auditors, and contributors verify
that the deployed binary matches the source.

### Prerequisites

Install Nix with flakes enabled:

```bash
# Install Nix (multi-user, recommended)
sh <(curl -L https://nixos.org/nix/install) --daemon

# Enable flakes (add to ~/.config/nix/nix.conf)
echo "extra-experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf
```

### Build from source

```bash
# Build the main server binary
nix build .#ethpayserver

# Build other binaries
nix build .#migrate-postgres
nix build .#evmmonitor
nix build .#ethpay-mcp

# Build the WASM client
nix build .#client
```

The output is a symlink at `./result` pointing to the Nix store path.

### Verify a build

```bash
# Build and note the store path
nix build .#ethpayserver --no-link --print-out-paths
# e.g. /nix/store/abc123...-ethpayserver-0.1.0

# On a second machine (or after garbage-collecting), rebuild:
nix build .#ethpayserver --rebuild --no-link --print-out-paths
# Should print the same store path — identical binary.
```

### Local development with payserver-commons

The flake pulls `payserver-commons` from GitLab by default. To use your local
checkout instead:

```bash
nix build .#ethpayserver --override-input payserver-commons path:../payserver-commons
```

### Run checks

```bash
# Run all flake checks (clippy, fmt, nextest)
nix flake check
```

## Contributing

Contributions are welcome! This project is in active development.

## License

MIT License

---

**Built with Rust** | **Self-hosted** | **Open Source**
