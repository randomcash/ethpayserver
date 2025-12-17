# ETHPayServer

A self-hosted Ethereum payment processor built in Rust.

Accept ETH and ERC20 tokens (USDC, USDT, DAI, WBTC, etc.) across 50+ EVM-compatible chains with a single codebase / infrastructure.

## Overview

ETHPayServer is a free, open-source payment processor that enables merchants to accept cryptocurrency payments on Ethereum and all EVM-compatible chains (Polygon, Arbitrum, Optimism, Base, BSC, and more).

### Key Features

- **Multi-Chain Support** - Works across EVM chains with one codebase
- **Native + Token Payments** - Accept ETH and whitelisted ERC20 tokens
- **Payment Monitoring** - Real-time detection of incoming payments
- **Reorg Protection** - Handles blockchain reorganizations safely
- **Gas Optimization** - EIP-1559 support with dynamic gas estimation
- **gRPC API** - Integrates with the Unified API Gateway

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
git clone https://github.com/your-org/ethpayserver.git
cd ethpayserver

# Copy environment config
cp .env.example .env
# Edit .env with your RPC URLs and database credentials

# Run database migrations
sqlx migrate run

# Build the project
cargo build --release

# Run tests
cargo test

# Start the server
cargo run --release
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
├── auth/              # User authentication (passkeys, wallets, recovery)
├── crypto/            # Cryptographic primitives (Argon2, AES, Ed25519)
├── data-service/      # PostgreSQL data access layer
├── evm/               # EVM blockchain interaction
├── types/             # Common types and traits
└── memos/             # Project documentation
```

## Crates

| Crate | Description |
|-------|-------------|
| [auth](./auth/README.md) | User authentication: passkeys, Ethereum wallets, BIP39 recovery |
| [crypto](./crypto/README.md) | Cryptographic primitives: Argon2id, AES-256, X25519, Ed25519 |
| [data-service](./data-service/README.md) | PostgreSQL repository implementations |
| [evm](./evm/README.md) | EVM blockchain: 10 networks, HD wallet, ERC20/721/1155 |
| [types](./types/README.md) | Common types shared across all payservers |

> **Note**: `auth`, `crypto`, and `types` will be moved to a shared `payserver-commons` repository.

## Development Status

**Current Phase:** Foundation Complete

This project is in active development. See the [project overview](./memos/project-overview.md) for detailed technical specifications and roadmap.

### What's Implemented

#### Infrastructure
- [x] Workspace structure with modular crates
- [x] PostgreSQL database with migrations
- [x] Repository pattern (Reader/Writer traits)

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

#### Database Schema
- [x] Users, sessions, devices, credentials
- [x] Stores, store_roles, user_stores
- [x] Invoices, payments, watched_addresses
- [x] Tokens table

### What's Remaining

#### Payment Processing
- [ ] Payment monitor (watch addresses for incoming payments)
- [ ] Gas estimator (EIP-1559 dynamic pricing)
- [ ] Reorg detector (handle chain reorganizations)

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
