# types

Common types and traits for the PayServer ecosystem.

> **Note**: This crate will be moved to `payserver-commons` repository.

## Purpose

This crate provides the foundation shared across all PayServer implementations (ethpayserver, bitcoinpayserver, etc.). It defines:

- **Network** - Enum of all supported blockchain networks
- **PayServer trait** - Core interface all payment servers implement
- **Data types** - `InvoiceData`, `PaymentData`, `TokenData`
- **Repository traits** - Database abstraction layer

## Modules

| Module | Description |
|--------|-------------|
| `types` | Core types: `Network`, `InvoiceId`, `InvoiceStatus`, `PaymentEvent` |
| `traits` | `PayServer` trait, `InvoiceData`, `PaymentData`, `CreateInvoiceRequest` |
| `repositories` | Database traits: `InvoiceRepository`, `PaymentRepository`, `TokenRepository` |
| `error` | `PayServerError` and `PayServerResult` |

## Repository Pattern

Each domain has Reader/Writer/Repository traits:

```rust
// Read-only access for API queries
fn list_invoices(reader: &impl InvoiceReader) { ... }

// Write access for processing
fn create_invoice(writer: &impl InvoiceWriter) { ... }

// Full access
fn process_payment(repo: &impl InvoiceRepository) { ... }
```

## Supported Networks

### EVM
- Ethereum, Polygon, Arbitrum, Optimism, Base
- Avalanche, BNB Smart Chain, zkSync, Linea, Scroll

### Bitcoin
- Bitcoin Mainnet, Bitcoin Testnet

### Lightning
- Lightning Network
