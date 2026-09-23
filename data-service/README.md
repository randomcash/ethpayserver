# data-service

Data access layer for ethpayserver.

## Features

| Feature | Description |
|---------|-------------|
| `postgres` (default) | PostgreSQL implementation |
| `redis` | Redis-backed event bridge persistence |
| `test-utils` | In-memory implementation for testing |

## Repository Traits

Implements the repository traits from `types` and `auth` — around twenty as
of this writing, not the seven below. A representative sample:

| Repository | Description |
|------------|-------------|
| `InvoiceRepository` | Invoice CRUD and queries |
| `PaymentRepository` | Payment records and confirmations |
| `TokenRepository` | Token configuration (ERC20, etc.) |
| `WatchedAddressRepository` | Address monitoring for payments |
| `StoreRepository` | Store CRUD and user access |
| `StoreRoleRepository` | Role definitions and permissions |
| `UserStoreRepository` | User-store membership management |
| `PayoutRepository`, `RefundRepository` | Payout and refund ledger records |
| `StoreTokenPolicyRepository` | Per-store token allowlist/blocklist |
| `WebhookDeliveryRepository` | Webhook delivery attempts |

See `src/postgres/` for the full list — one file per repository, roughly.

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

Not exhaustive — `migrations/postgres/` is the source of truth; this repo's
root README has a fuller (still partial) table-by-table breakdown.

### Core Tables
- `invoices` - Payment invoices
- `payments` - Detected payments
- `watched_addresses` - Addresses being monitored
- `tokens` - Configured tokens (ERC20, ERC721, etc.)
- `payouts`, `refunds` - Payout and (historical) refund ledger records

### Auth Tables
- `users` - User accounts
- `sessions` - Active sessions
- `devices` - Registered devices/passkeys
- `wallets` - Account receiving wallets, one per chain family (`eip155`,
  `tron`) a store has funded — not Ethereum-only. The `tron` family is
  derivation/storage plumbing only: no Tron chain is enabled or monitored
  today, so the API refuses to create a `tron:` payment method until an
  adapter exists (`server/src/api/stores/payment_methods.rs`)

### Store Tables (Multi-Tenant)
- `stores` - Merchant stores
- `store_roles` - Role definitions with permissions (Owner, Manager, Employee, Guest)
- `user_stores` - Links users to stores with roles
- `store_token_policies`, `store_token_policy_entries` - Per-store token
  allowlist/blocklist

```sql
-- Default store roles are seeded on migration
-- Store-specific roles have store_id set, global defaults have NULL
INSERT INTO store_roles (store_id, role, permissions) VALUES
    (NULL, 'Owner', '["ethpay.store.canmodifystoresettings", ...]'),
    (NULL, 'Manager', '["ethpay.store.canviewinvoices", ...]'),
    ...
```

## Structure

```
data-service/
├── migrations/
│   └── postgres/       # PostgreSQL migrations
├── src/
│   ├── lib.rs
│   ├── postgres/        # PostgreSQL implementations (one file per repository, roughly)
│   │   ├── auth/         # Auth-specific repository implementations
│   │   └── integration_tests/  # #[ignore]'d tests against a real Postgres
│   ├── redis/            # Redis-backed event bridge persistence (feature: redis)
│   ├── invoice_creation.rs, payout_claims.rs, reorg.rs, analytics.rs,
│   │   store_creation.rs, account_deletion.rs, and other operations that
│   │   span more than one repository
│   └── test_utils.rs    # In-memory test implementation
└── Cargo.toml
```
