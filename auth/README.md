# auth

User authentication, roles, permissions, and device management for PayServer.

> **Note**: This crate will be moved to `payserver-commons` repository.

## Features

- **Passkeys** - Phishing-resistant, passwordless authentication (WebAuthn)
- **Ethereum Wallets** - Web3-native authentication via EIP-191 signatures
- **BIP39 Recovery** - Mnemonic-based account recovery
- **Zero-Knowledge Storage** - Server stores only encrypted blobs it cannot decrypt
- **Role-Based Access Control** - Hierarchical permissions at server and store levels
- **Multi-Tenant Stores** - Users can belong to multiple stores with different roles

## Architecture

```
Client                                    Server
──────                                    ──────
BIP39 Mnemonic + Identifier (email or wallet)
      │
      ▼ Argon2id
Recovery Key ──────────────────────────► recovery_verification_hash
      │
      ▼ Encrypt
Encrypted Symmetric Key ───────────────► Stored (user can decrypt)

Passkey ───────────────────────────────► Stored (for authentication)
Wallet Signature ──────────────────────► Verified (EIP-191 personal_sign)
```

## Usage

Mount the axum routes at `/auth`:

```rust
use auth::{api, AuthService};
use std::sync::Arc;

let service = Arc::new(AuthService::new(repo));
let state = api::AuthState::new(service);

let app = Router::new()
    .nest("/auth", api::router(state));
```

## Authentication Flows

### Email + Passkey
1. User registers with email + passkey + mnemonic
2. User authenticates with passkey (Touch ID, Face ID, etc.)
3. If passkeys lost, recover with mnemonic

### Wallet-Only (Web3)
1. User registers with wallet signature + mnemonic
2. User signs challenge message with wallet
3. If wallet lost, recover with mnemonic

## Dependencies

- `crypto` - Cryptographic primitives
- `types` - Common types (Store, UserId)
- `data-service` - User/session/device persistence

## Roles and Permissions

### Server Roles

| Role | Description |
|------|-------------|
| `ServerAdmin` | Full access to all server operations |
| `User` | Standard user with access to own profile and stores |

### Permission Scopes

Permissions are organized into three scopes:

| Scope | Prefix | Example |
|-------|--------|---------|
| Server | `ethpay.server.*` | `ethpay.server.canmodifyserversettings` |
| Store | `ethpay.store.*` | `ethpay.store.cancreateinvoice` |
| User | `ethpay.user.*` | `ethpay.user.canviewprofile` |

### Store Roles

Users can have different roles in each store they belong to:

| Role | Description |
|------|-------------|
| `Owner` | Full access to store settings, users, invoices, payments |
| `Manager` | Can manage most store settings and view all data |
| `Employee` | Can view and create invoices |
| `Guest` | Read-only access to invoices |

### Permission Model

```
User ──── Role (ServerAdmin/User)
  │
  └──── UserStore ──── Store
              │
              └── StoreRole ──── Permissions[]
```

- **Server roles** define baseline capabilities
- **Store roles** add store-specific permissions
- A user's effective permissions = server role + store roles for accessed store
