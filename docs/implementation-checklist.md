# EthPayServer Implementation Checklist

## Phase 0: Encryption Foundation (COMPLETED 2024-12-13)

> Note: Implemented as `crypto` crate within `ethpayserver/` workspace

- [x] Add `crypto` crate to ethpayserver workspace
- [x] Argon2id key derivation (RFC 9106) - `kdf.rs`
- [x] HKDF-SHA256 key stretching (RFC 5869) - `kdf.rs`
- [x] AES-256-CBC + HMAC-SHA256 encryption - `symmetric.rs`
- [x] X25519 key generation/exchange (RFC 7748) - `asymmetric.rs`
- [x] Ed25519 signing (RFC 8032) - `asymmetric.rs`
- [x] BIP39 mnemonic generation/validation - `mnemonic.rs`
- [x] `EncryptedBlob`, `KdfParams` types - `types.rs`
- [x] Unit tests for all crypto operations (40 tests passing)
- [x] Security audit: zeroization, timing attacks, dependency CVEs

## Phase 0.5: User & Device Management (COMPLETED 2024-12-14)

> Note: Implemented as `auth` crate within `ethpayserver/` workspace

### Core Library (COMPLETED 2024-12-14)
- [x] `User` model (encrypted symmetric key, recovery hash, optional email/wallet)
- [x] `Device` model (device-encrypted symmetric key)
- [x] `Session` model (session management with idle timeout)
- [x] Repository traits (UserRepository, DeviceRepository, SessionRepository, PasskeyRepository, WalletRepository, ChallengeRepository)
- [x] InMemoryRepository for testing
- [x] AuthService with business logic:
  - [x] **Passkey Authentication (WebAuthn)** - phishing-resistant, passwordless
    - [x] start_new_user_passkey_registration() / complete_new_user_passkey_registration()
    - [x] start_passkey_login() / complete_passkey_login()
    - [x] start_passkey_registration() / complete_passkey_registration() (add passkey to existing user)
    - [x] get_passkeys() / revoke_passkey()
  - [x] **Ethereum Wallet Authentication (EIP-191)** - Web3-native auth
    - [x] start_wallet_login() / complete_wallet_login()
    - [x] start_new_user_wallet_registration() / complete_new_user_wallet_registration()
    - [x] start_wallet_registration() / complete_wallet_registration() (add wallet to existing user)
    - [x] get_wallets() / revoke_wallet()
    - [x] EIP-55 address checksumming (alloy-primitives)
    - [x] EIP-191 personal_sign signature verification (k256, sha3)
  - [x] **Account Recovery** - BIP39 mnemonic flow
    - [x] start_account_recovery() / complete_account_recovery()
    - [x] Supports email OR wallet address as identifier
  - [x] validate_session() - session validation with idle timeout
  - [x] logout() / logout_all() - session termination
  - [x] get_devices() / revoke_device() - device management
- [x] Account lockout protection (configurable max attempts + duration)
- [x] User enumeration prevention (generic error messages)
- [x] Constant-time comparison for recovery hash verification
- [x] Zeroize sensitive data on drop
- [x] Unit tests (23 tests passing, including cryptographic signature verification)

### Database (COMPLETED 2024-12-14)
- [x] PostgreSQL repository implementation (`data-service/src/postgres.rs`)
- [x] Database migrations created (`data-service/migrations/postgres/`):
  - [x] `20241214000001_create_auth_tables.sql` - users, devices, sessions, passkeys, wallets, challenges
  - [x] `20241214000002_create_payment_tables.sql` - invoices, payments, watched_addresses, events

### Data Service Hardening (COMPLETED 2024-12-15)
- [x] Added `InvalidData` error variant to `RepositoryError` for conversion failures
- [x] Conversion functions now return `Result` instead of silent fallbacks:
  - [x] `network_to_db` → `try_network_to_db` (rejects Bitcoin networks)
  - [x] `db_to_network` → `try_db_to_network` (rejects unknown values)
  - [x] `db_to_status` → `try_db_to_status` (rejects unknown values)
- [x] Added `rows_affected` checks to update operations:
  - [x] `invoice.rs`: `update_status`, `update_amount_received`
  - [x] `payment.rs`: `update_confirmations`
  - [x] `watched_address.rs`: `remove`
- [x] Fixed race condition in `watched_address.rs` upsert with transaction + `SELECT FOR UPDATE`
- [x] Removed unnecessary `format!()` calls in `auth/challenge.rs` cleanup
- [x] Integration tests: 21 passing
- [x] Unit tests: 13 passing

### API Endpoints (COMPLETED 2024-12-14)

> Note: Implemented in `auth/src/api/` module

- [x] POST /auth/passkey/new-user/start
- [x] POST /auth/passkey/new-user/complete
- [x] POST /auth/passkey/register/start
- [x] POST /auth/passkey/register/complete
- [x] POST /auth/passkey/login/start
- [x] POST /auth/passkey/login/complete
- [x] POST /auth/wallet/new-user/start
- [x] POST /auth/wallet/new-user/complete
- [x] POST /auth/wallet/register/start
- [x] POST /auth/wallet/register/complete
- [x] POST /auth/wallet/login/start
- [x] POST /auth/wallet/login/complete
- [x] POST /auth/recovery/start
- [x] POST /auth/recovery/complete
- [x] GET /auth/devices
- [x] DELETE /auth/devices/:id
- [x] GET /auth/passkeys
- [x] DELETE /auth/passkeys/:id
- [x] GET /auth/wallets
- [x] DELETE /auth/wallets/:id
- [x] POST /auth/logout
- [x] POST /auth/logout/all

## Phase 1: payserver-common (COMPLETED 2024-12-13)

> Note: Implemented as `types` crate within `ethpayserver/` workspace

- [x] Create workspace Cargo.toml
- [x] Create types crate (`ethpayserver/types/`)
- [x] Define Amount, Currency types
- [x] Define InvoiceId, Invoice types
- [x] Define Payment, PaymentMethod, PaymentEvent types
- [x] Define InvoiceStatus, HealthStatus types
- [x] Define PayServer trait
- [x] Define PaymentMonitor trait
- [x] Define PayServerError
- [x] Add unit tests (11 tests passing)
- [x] Documentation comments
- [x] Create data-service crate with DataService trait
- [x] InMemoryDataService for testing

## Phase 2: ethpayserver Foundation

- [x] Set up Cargo.toml with workspace dependencies
- [ ] Create directory structure (core/, chains/, api/, db/)
- [ ] Define EVMChain trait
- [ ] Define EthPayError
- [ ] Implement config loading
- [x] Create database migrations
- [ ] Implement health check endpoint

## Phase 3: ethpayserver Core

- [ ] Implement Ethereum chain
- [ ] HD wallet address derivation
- [ ] Native ETH balance checking
- [ ] Payment monitoring loop
- [ ] Basic REST API (create invoice, get status)

## Phase 4: ethpayserver Tokens

- [ ] ERC20 token registry
- [ ] Token balance checking
- [ ] Transfer event parsing
- [ ] Multi-token invoice support
