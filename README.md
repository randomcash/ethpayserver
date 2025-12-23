# ETHPayServer

A self-hosted Ethereum payment processor built in Rust.

Accept ETH and ERC20 tokens (USDC, USDT, DAI, WBTC, etc.) across 50+ EVM-compatible chains with a single codebase / infrastructure.

## Overview

ETHPayServer is a free, open-source payment processor that enables merchants to accept cryptocurrency payments on Ethereum and all EVM-compatible chains (Polygon, Arbitrum, Optimism, Base, BSC, and more).

### Key Features

- **Multi-Chain Support** - Works across 10+ EVM chains with one codebase
- **Native + Token Payments** - Accept ETH and whitelisted ERC20 tokens
- **Payment Monitoring** - Real-time detection via WebSocket subscriptions
- **Reorg Protection** - Handles blockchain reorganizations safely
- **Horizontal Scaling** - Separate monitor binary with Redis event bridge
- **Direct RPC Support** - Connect to your own nodes or use providers (Alchemy, Infura)

### Supported Chains

- Ethereum (mainnet)
- Polygon
- Arbitrum
- Optimism
- Base
- Binance Smart Chain
- And 40+ more EVM-compatible chains

### Supported Tokens

- Native tokens (ETH, MATIC, etc.)
- USDC
- USDT
- DAI
- WBTC
- And more whitelisted ERC20 tokens

## Technology Stack

- **Language:** Rust (stable, latest)
- **Ethereum Client:** ethers-rs / alloy
- **Async Runtime:** tokio
- **Web Framework:** axum (REST) / tonic (gRPC)
- **Database:** PostgreSQL with sqlx
- **Error Handling:** anyhow, thiserror
- **Logging:** tracing, tracing-subscriber

## Architecture

The project follows a modular architecture:

- **Core** - EVM chain abstraction, wallet handling, token management
- **Chains** - Individual chain implementations (one per EVM chain)
- **Monitoring** - Payment detection, gas estimation, reorg handling
- **API** - gRPC and REST endpoints
- **Database** - PostgreSQL for invoice and payment tracking

## Prerequisites

- Rust 1.75+
- PostgreSQL 14+
- Ethereum RPC access (Alchemy, Infura, or self-hosted node)

## Quick Start

```bash
# Clone the repository
git clone git@gitlab.com:random.cash/ethpayserver.git
cd ethpayserver

# Copy environment config
cp .env.example .env
# Edit .env with your RPC URLs and database credentials

# Run database migrations
cargo install sqlx-cli
sqlx migrate run

# Build the project
cargo build --release

# Run tests
cargo test

# Start the API server
cargo run --release -p core
```

### Running the Monitor

The payment monitor runs as a separate binary and uses **WebSocket for real-time block subscriptions**:

```bash
# Build monitor binary
cargo build --release --bin evmmonitor --features monitor-bin

# Run with environment variables (WebSocket URLs required for real-time monitoring)
EVMMONITOR_REDIS_URL=redis://localhost:6379 \
EVMMONITOR_CHAINS=1,137,42161 \
EVMMONITOR_CHAIN_1_RPC_HTTP=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
EVMMONITOR_CHAIN_1_RPC_WS=wss://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
EVMMONITOR_CHAIN_137_RPC_HTTP=https://polygon-mainnet.g.alchemy.com/v2/YOUR_KEY \
EVMMONITOR_CHAIN_137_RPC_WS=wss://polygon-mainnet.g.alchemy.com/v2/YOUR_KEY \
EVMMONITOR_CHAIN_42161_RPC_HTTP=https://arb-mainnet.g.alchemy.com/v2/YOUR_KEY \
EVMMONITOR_CHAIN_42161_RPC_WS=wss://arb-mainnet.g.alchemy.com/v2/YOUR_KEY \
./target/release/evmmonitor
```

Or with a config file:

```bash
# Create evmmonitor.toml (see docs for full options)
cat > evmmonitor.toml << EOF
[bridge]
redis_url = "redis://localhost:6379"

[[chains]]
chain_id = 1
rpc_http = "https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY"
rpc_ws = "wss://eth-mainnet.g.alchemy.com/v2/YOUR_KEY"
EOF

./target/release/evmmonitor --config evmmonitor.toml
```

## Docker

```bash
# Build image
docker build -t ethpayserver .

# Run container
docker run -p 5001:5001 -p 5002:5002 \
  -e DATABASE_URL=postgres://... \
  -e ETHEREUM_RPC_URL=https://... \
  ethpayserver
```

## Project Structure

```
ethpayserver/
├── core/              # Main API server binary
├── data-service/      # PostgreSQL data access layer
├── evm/               # EVM blockchain interaction + monitor binary
│   └── src/
│       ├── bin/
│       │   └── evmmonitor.rs   # Standalone chain monitor
│       └── monitor/
│           ├── bridge/         # Event bridge (Redis/Memory)
│           ├── source/         # Block sources (RPC, WS)
│           ├── chain.rs        # Per-chain monitor
│           └── coordinator.rs  # Multi-chain coordinator
└── memos/             # Project documentation
```

## Crates & Binaries

| Crate | Binary | Description |
|-------|--------|-------------|
| core | `ethpayserver` | Main API server: REST API, Swagger UI |
| evm | `evmmonitor` | Chain monitor: watches blocks, publishes events to Redis |
| data-service | - | PostgreSQL repository implementations |

### External Dependencies

Shared libraries from [payserver-commons](https://gitlab.com/random.cash/payserver-commons):

| Crate | Description |
|-------|-------------|
| auth | User authentication: passkeys, Ethereum wallets, BIP39 recovery |
| crypto | Cryptographic primitives: Argon2id, AES-256, X25519, Ed25519 |
| types | Common types shared across all payservers |

## Development Status

**Current Phase:** Payment Monitoring

This project is in active development. See the [project overview](./memos/project-overview.md) for detailed technical specifications and roadmap.

### What's Implemented

#### Infrastructure
- [x] Workspace structure with modular crates
- [x] PostgreSQL database with migrations
- [x] Repository pattern (Reader/Writer traits)
- [x] Unified API server (`core` crate)
- [x] Health check endpoints
- [x] Swagger UI / OpenAPI documentation
- [x] Shared libraries extracted to `payserver-commons`

#### Authentication (`auth` crate)
- [x] Passkey/WebAuthn authentication
- [x] Ethereum wallet authentication (EIP-191)
- [x] BIP39 mnemonic recovery
- [x] Session and device management
- [x] Role-based permissions (ServerAdmin, User)
- [x] Multi-tenant stores with StoreRoles

#### EVM Support (`evm` crate)
- [x] 10 EVM networks configured (Ethereum, Polygon, Arbitrum, etc.)
- [x] HD wallet derivation (BIP-32/44)
- [x] RPC provider abstraction (Alloy)
- [x] Token management API (CRUD + enable/disable)
- [x] Admin authentication on API endpoints

#### Payment Monitoring (`evmmonitor` binary)
- [x] Standalone chain monitor binary
- [x] WebSocket block subscriptions (real-time)
- [x] HTTP polling fallback
- [x] Direct RPC and provider support (Alchemy, Infura)
- [x] Redis event bridge for horizontal scaling
- [x] Multi-chain support (run multiple instances)
- [x] Native ETH and ERC20 transfer detection
- [x] Confirmation tracking
- [x] Reorg detection

#### Database Schema
- [x] Users, sessions, devices, credentials
- [x] Stores, store_roles, user_stores
- [x] Invoices, payments, watched_addresses
- [x] Tokens table

### What's Remaining

#### Payment Processing
- [ ] API server event subscription (consume Redis events)
- [ ] Invoice status updates from monitor events
- [ ] Gas estimator (EIP-1559 dynamic pricing)

#### API & Integration
- [ ] gRPC API for gateway integration
- [ ] Invoice creation workflow
- [ ] Webhook notifications
- [ ] Exchange rate feeds

#### Production Readiness
- [ ] Prometheus metrics
- [ ] Health checks per chain
- [ ] Load testing
- [ ] Security audit

## Documentation

The `/docs` directory contains development notes, technical specifications, and AI-generated content used during the development process. These files provide context and planning documentation for the project.

## Security

ETHPayServer implements several security measures:

- Address validation (checksum verification)
- Whitelisted token contracts only
- Confirmation requirements per chain
- Reorg detection and handling
- Rate limiting on RPC calls
- Audit logging for all payment events

## Performance Targets

- Invoice creation: <100ms
- Payment detection: <5 seconds after confirmation
- Concurrent monitored addresses: 10,000+
- Uptime: 99.9%

## Contributing

Contributions are welcome! This project is in early development. Please check back soon for contribution guidelines.

## License

[To be determined]

## Support

For questions and support, please open an issue on GitHub.

---

**Built with Rust** | **Self-hosted** | **Open Source**
