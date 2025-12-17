# evm

EVM blockchain interaction layer for ethpayserver.

## Features

- **Network configs** - Pre-defined settings for 10 EVM networks
- **HD wallet** - BIP-32/44 address derivation for payment invoices
- **RPC provider** - Alloy-based provider for blockchain interaction
- **Token support** - ERC20, ERC721, ERC1155 standards

## Supported Networks

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

## Usage

```rust
use evm::{EvmNetwork, HdWallet, EvmProvider};

// Create wallet from mnemonic
let wallet = HdWallet::from_mnemonic("your mnemonic here", "")?;

// Derive payment address
let address = wallet.derive_address(0)?;

// Connect to network
let provider = EvmProvider::new(EvmNetwork::Ethereum, "https://eth.llamarpc.com").await?;

// Check balance
let balance = provider.get_native_balance(address).await?;
```

## Modules

| Module | Description |
|--------|-------------|
| `network` | `EvmNetwork` enum and `ChainConfig` |
| `wallet` | BIP-32/44 HD wallet derivation |
| `provider` | Alloy RPC provider wrapper |
| `tokens` | `EvmTokenStandard`, ERC20 operations |
| `error` | `EvmError` and `EvmResult` |
| `api` | REST API endpoints (feature: `api`) |

## Token Standards

```rust
use evm::EvmTokenStandard;

let standard: EvmTokenStandard = "erc20".parse()?;
standard.validate(Some(18), None)?; // ERC20 needs decimals, no token_id
```

## REST API (feature: `api`)

Enable with `--features api`. Provides axum routes for token and network management.

```rust
use evm::api::{EvmState, router};
use data_service::PgDataService;
use std::sync::Arc;

let ds = Arc::new(PgDataService::connect("postgres://...").await?);
let app = Router::new().nest("/evm", router(EvmState::new(ds)));
```

### Token Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/evm/tokens` | List tokens (filter by network, type, symbol) |
| POST | `/evm/tokens` | Create token |
| GET | `/evm/tokens/{id}` | Get token |
| PUT | `/evm/tokens/{id}` | Update token |
| DELETE | `/evm/tokens/{id}` | Delete token |
| PUT | `/evm/tokens/{id}/enabled` | Enable/disable token |

### Network Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/evm/networks` | List all networks |
| GET | `/evm/networks/{id}` | Get network info |
