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

## Token Standards

```rust
use evm::EvmTokenStandard;

let standard: EvmTokenStandard = "erc20".parse()?;
standard.validate(Some(18), None)?; // ERC20 needs decimals, no token_id
```
