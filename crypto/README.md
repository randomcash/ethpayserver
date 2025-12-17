# crypto

Cryptographic primitives for PayServer.

> **Note**: This crate will be moved to `payserver-commons` repository.

## Features

Implements Bitwarden-style client-side encryption where the server never sees plaintext data.

- **KDF** - Argon2id for password hashing, HKDF-SHA256 for key derivation
- **Symmetric** - AES-256-CBC + HMAC-SHA256 authenticated encryption
- **Asymmetric** - X25519 key exchange, Ed25519 signatures
- **Mnemonic** - BIP39 recovery phrase generation and validation

## Key Hierarchy

```
Password + Email
       |
       v (Argon2id)
  Master Key (256-bit)
       |
  +----+----+
  |         | (HKDF)
  v         v
enc_key   mac_key   <- StretchedKey (512-bit)
  |
  v (encrypt)
[Encrypted Symmetric Key] <- stored on server
  |
  v (decrypt with stretched key)
Symmetric Key (256-bit, random)
  |
  v (AES-256-CBC + HMAC-SHA256)
[Encrypted User Data]
```

## Usage

```rust
use crypto::{kdf, symmetric, types::KdfParams};

// Derive master key from password
let params = KdfParams::new_random();
let master_key = kdf::derive_master_key(b"password", "user@example.com", &params)?;

// Stretch to get encryption and MAC keys
let stretched = kdf::stretch_master_key(&master_key)?;

// Encrypt data
let blob = symmetric::encrypt(b"Secret data", &stretched)?;

// Decrypt
let plaintext = symmetric::decrypt(&blob, &stretched)?;
```

## Modules

| Module | Description |
|--------|-------------|
| `kdf` | Key derivation (Argon2id, HKDF-SHA256) |
| `symmetric` | Authenticated encryption (AES-256-CBC + HMAC-SHA256) |
| `asymmetric` | Key exchange and signing (X25519, Ed25519) |
| `mnemonic` | BIP39 recovery phrases |
| `types` | Core cryptographic types |
