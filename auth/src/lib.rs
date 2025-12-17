//! User authentication and device management for PayServer.
//!
//! # HTTP API
//!
//! This crate provides ready-to-use axum routes. Mount them at `/auth`:
//!
//! ```rust,ignore
//! use auth::{api, AuthService};
//! use std::sync::Arc;
//!
//! let service = Arc::new(AuthService::new(repo));
//! let state = api::AuthState::new(service);
//!
//! let app = Router::new()
//!     .nest("/auth", api::router(state));
//! ```
//!
//! This crate provides multiple authentication methods with BIP39 mnemonic recovery:
//! - **Passkeys** for phishing-resistant, passwordless authentication
//! - **Ethereum Wallets** for Web3-native authentication (MetaMask, etc.)
//! - **BIP39 mnemonic** for account recovery (required)
//! - Server stores only encrypted blobs it cannot decrypt
//!
//! # Architecture
//!
//! ```text
//! Client                                    Server
//! ──────                                    ──────
//! BIP39 Mnemonic + Identifier (email or wallet)
//!       │
//!       ▼ Argon2id
//! Recovery Key ──────────────────────────► recovery_verification_hash
//!       │                                   (for recovery verification)
//!       ▼ Encrypt
//! Encrypted Symmetric Key ───────────────► Stored (user can decrypt)
//!
//! Passkey ───────────────────────────────► Stored (for authentication)
//! Wallet Signature ──────────────────────► Verified (EIP-191 personal_sign)
//! ```
//!
//! # Authentication Flows
//!
//! ## Email + Passkey (Traditional)
//! 1. **Registration**: User creates account with email + passkey + mnemonic
//! 2. **Login**: User authenticates with passkey (Touch ID, Face ID, etc.)
//! 3. **Recovery**: If passkeys are lost, user can recover with mnemonic
//!
//! ## Wallet-Only (Web3)
//! 1. **Registration**: User creates account with wallet signature + mnemonic
//! 2. **Login**: User signs challenge message with wallet
//! 3. **Recovery**: User recovers with mnemonic (salt is "wallet:{address}")
//!
//! # Usage
//!
//! ```rust,ignore
//! use auth::{AuthService, AuthConfig};
//!
//! // Create service with your repository implementation
//! let repo = Arc::new(MyDatabaseRepo::new(pool));
//! let service = AuthService::new(repo);
//!
//! // Start new user registration (returns challenge + user_id)
//! let start_response = service.start_new_user_passkey_registration(&email).await?;
//!
//! // Complete registration with passkey credential (user_id included in request)
//! let response = service.complete_new_user_passkey_registration(request).await?;
//!
//! // Login with passkey
//! let challenge = service.start_passkey_login(&email).await?;
//! let response = service.complete_passkey_login(request).await?;
//!
//! // Validate session
//! let (user, session) = service.validate_session(session_id).await?;
//! ```

pub mod api;
pub mod error;
pub mod models;
pub mod permissions;
pub mod repository;
pub mod service;
pub mod store;

// Re-export OpenAPI doc
pub use api::AuthApiDoc;

// Re-export main types
pub use error::{AuthError, Result};
pub use models::{
    // User/Device/Session types
    Device, DeviceId, DeviceInfo, DeviceType, LoginResponse, Session, SessionId, User, UserId,
    UserInfo,
    // Passkey types (primary authentication)
    CompleteNewUserPasskeyRegistrationRequest, CompletePasskeyLoginRequest,
    CompletePasskeyRegistrationRequest, PasskeyCredential, PasskeyId, PasskeyInfo,
    StartNewUserPasskeyRegistrationResponse, StartPasskeyLoginResponse,
    StartPasskeyRegistrationRequest, StartPasskeyRegistrationResponse,
    // Wallet types (Ethereum wallet authentication)
    CompleteNewUserWalletRegistrationRequest, CompleteWalletLoginRequest,
    CompleteWalletRegistrationRequest, StartNewUserWalletRegistrationRequest,
    StartNewUserWalletRegistrationResponse, StartWalletLoginRequest, StartWalletLoginResponse,
    StartWalletRegistrationRequest, StartWalletRegistrationResponse, WalletChallenge,
    WalletCredential, WalletCredentialId, WalletInfo,
    // Recovery types
    CompleteRecoveryRequest, StartRecoveryRequest,
    // WebAuthn re-exports (for client use and repository implementations)
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
};
pub use repository::{
    AuthRepository, ChallengeRepository, DeviceRepository, PasskeyRepository, SessionRepository,
    StoreRepository, StoreRoleRepository, UserRepository, UserStoreRepository, WalletRepository,
};
pub use service::{AuthConfig, AuthService};
pub use permissions::{Permission, Policies, Role};
pub use store::{
    Store, StoreId, StoreInfo, StoreRole, StoreRoleId, StoreRoleInfo, UserStore, UserStoreInfo,
    default_roles,
};
