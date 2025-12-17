# core

Main server for ETHPayServer - unified API and service orchestration.

## Overview

The core crate provides the main `ethpayserver` binary that combines all other crates into a single HTTP server with a unified API.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     ETHPayServer Core                       │
├─────────────────────────────────────────────────────────────┤
│  API Layer (axum)                                           │
│  ├── /health     - Health checks                            │
│  ├── /evm        - EVM operations (tokens, networks)        │
│  ├── /auth       - Authentication (TODO)                    │
│  └── /swagger-ui - API documentation                        │
├─────────────────────────────────────────────────────────────┤
│  Service Layer                                              │
│  ├── AuthService   - User authentication & sessions         │
│  └── (PaymentService, InvoiceService - TODO)                │
├─────────────────────────────────────────────────────────────┤
│  Data Layer                                                 │
│  └── PgDataService - PostgreSQL repositories                │
└─────────────────────────────────────────────────────────────┘
```

## Running

```bash
# Set required environment variables
export DATABASE_URL="postgres://user:pass@localhost/ethpayserver"

# Optional configuration
export HOST="0.0.0.0"      # Default: 127.0.0.1
export PORT="3000"         # Default: 3000
export LOG_LEVEL="info"    # Default: info
export ENABLE_SWAGGER="true"  # Default: true

# Run the server
cargo run --release --bin ethpayserver
```

## API Endpoints

### Health

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Full health check with database status |
| `GET /health/live` | Liveness probe (always 200 if running) |
| `GET /health/ready` | Readiness probe (checks database) |

### EVM (mounted at `/evm`)

| Endpoint | Description |
|----------|-------------|
| `GET /evm/networks` | List supported networks |
| `GET /evm/networks/{id}` | Get network details |
| `GET /evm/tokens` | List tokens (admin) |
| `POST /evm/tokens` | Create token (admin) |
| `GET /evm/tokens/{id}` | Get token (admin) |
| `PUT /evm/tokens/{id}` | Update token (admin) |
| `DELETE /evm/tokens/{id}` | Delete token (admin) |

## Swagger UI

When `ENABLE_SWAGGER=true`, the Swagger UI is available at `/swagger-ui`.

## Dependencies

- `auth` - Authentication service
- `data-service` - PostgreSQL data access
- `evm` - EVM blockchain operations
- `types` - Common types
