# data-service

Data access layer for EthPayServer. Provides database implementations for the repository traits defined in the `types` crate.

## Features

| Feature | Description |
|---------|-------------|
| `postgres` (default) | PostgreSQL implementation |
| `test-utils` | In-memory implementation for testing |

## Usage

```rust
use data_service::{PgDataService, InvoiceReader, InvoiceWriter};
use sqlx::postgres::PgPoolOptions;

let pool = PgPoolOptions::new()
    .connect("postgres://user:pass@localhost/dbname")
    .await?;

let ds = PgDataService::new(pool);

// ds implements DataService (InvoiceRepository + PaymentRepository + WatchedAddressRepository)
```

## Migrations

Each database implementation has its own migrations directory under `migrations/`.

### PostgreSQL

```bash
# Run migrations
sqlx migrate run --source migrations/postgres

# Revert last migration
sqlx migrate revert --source migrations/postgres

# Check migration status
sqlx migrate info --source migrations/postgres
```

### Adding new migrations

```bash
sqlx migrate add -r <name> --source migrations/postgres
```

This creates both `.sql` (up) and `.down.sql` (down) migration files.

## Structure

```
data-service/
├── migrations/
│   └── postgres/       # PostgreSQL migrations
├── src/
│   ├── lib.rs
│   ├── postgres.rs     # PostgreSQL implementation
│   └── test_utils.rs   # In-memory test implementation
└── Cargo.toml
```
