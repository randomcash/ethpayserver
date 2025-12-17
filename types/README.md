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
| `types` | Core types: `Network`, `InvoiceId`, `InvoiceStatus`, `PaymentEvent`, `UserId` |
| `store` | Multi-tenant types: `Store`, `StoreId`, `StoreInfo` |
| `traits` | `PayServer` trait, `InvoiceData`, `PaymentData`, `CreateInvoiceRequest` |
| `repositories` | Database traits: `InvoiceRepository`, `PaymentRepository`, `TokenRepository` |
| `error` | `PayServerError` and `PayServerResult` |

## Store Types

The `store` module provides multi-tenant store support:

```rust
use types::{Store, StoreId, StoreInfo, UserId};

// Create a store
let owner_id = UserId::new();
let store = Store::new("My Shop", owner_id)
    .with_website("https://myshop.com");

// StoreInfo for API responses (excludes owner_id)
let info: StoreInfo = (&store).into();
```

| Type | Description |
|------|-------------|
| `StoreId` | UUID wrapper for store identification |
| `Store` | Full store entity with owner_id |
| `StoreInfo` | Sanitized store data for API responses |
| `UserId` | UUID wrapper for user identification |

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

## Features

| Feature | Description |
|---------|-------------|
| `openapi` | Adds `utoipa::ToSchema` derives for OpenAPI documentation |
