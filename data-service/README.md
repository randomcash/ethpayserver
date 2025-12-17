# data-service

Data access layer for ethpayserver.

## Features

| Feature | Description |
|---------|-------------|
| `postgres` (default) | PostgreSQL implementation |
| `test-utils` | In-memory implementation for testing |

## Repository Traits

Implements the repository traits from `types`:

| Repository | Description |
|------------|-------------|
| `InvoiceRepository` | Invoice CRUD and queries |
| `PaymentRepository` | Payment records and confirmations |
| `TokenRepository` | Token configuration (ERC20, etc.) |
| `WatchedAddressRepository` | Address monitoring for payments |

## Usage

```rust
use data_service::PgDataService;
use types::{InvoiceReader, InvoiceWriter, InvoiceStatus};

// Connect to PostgreSQL
let service = PgDataService::connect("postgres://...").await?;

// Read/write using repository traits
let invoice = InvoiceReader::get(&service, &invoice_id).await?;
InvoiceWriter::update_status(&service, &invoice_id, InvoiceStatus::Paid).await?;
```

## Migrations

```bash
# Run migrations
sqlx migrate run --source migrations/postgres

# Revert last migration
sqlx migrate revert --source migrations/postgres

# Add new migration
sqlx migrate add -r <name> --source migrations/postgres
```

## Testing

Enable `test-utils` for in-memory implementations:

```toml
[dev-dependencies]
data-service = { path = "../data-service", features = ["test-utils"] }
```

```rust
use data_service::test_utils::InMemoryDataService;

let ds = InMemoryDataService::new();
// Uses same traits as production
```

## Database Schema

### Core Tables
- `invoices` - Payment invoices
- `payments` - Detected payments
- `watched_addresses` - Addresses being monitored
- `tokens` - Configured tokens (ERC20, ERC721, etc.)

### Auth Tables
- `users` - User accounts
- `sessions` - Active sessions
- `devices` - Registered devices/passkeys
- `wallets` - Linked Ethereum wallets

## Structure

```
data-service/
├── migrations/
│   └── postgres/       # PostgreSQL migrations
├── src/
│   ├── lib.rs
│   ├── postgres/       # PostgreSQL implementations
│   └── test_utils.rs   # In-memory test implementation
└── Cargo.toml
```
