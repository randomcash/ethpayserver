//! User authentication and device management for PayServer.
//!
//! This crate provides passkey-only authentication with BIP39 mnemonic recovery:
//! - **Passkeys** for phishing-resistant, passwordless authentication
//! - **BIP39 mnemonic** for account recovery (required)
//! - Server stores only encrypted blobs it cannot decrypt
//!
//! # Architecture
//!
//! ```text
//! Client                                    Server
//! ──────                                    ──────
//! BIP39 Mnemonic + Email
//!       │
//!       ▼ Argon2id
//! Recovery Key ──────────────────────────► recovery_verification_hash
//!       │                                   (for recovery verification)
//!       ▼ Encrypt
//! Encrypted Symmetric Key ───────────────► Stored (user can decrypt)
//!
//! Passkey ───────────────────────────────► Stored (for authentication)
//! ```
//!
//! # Authentication Flow
//!
//! 1. **Registration**: User creates account with passkey + mnemonic
//! 2. **Login**: User authenticates with passkey (Touch ID, Face ID, etc.)
//! 3. **Recovery**: If passkeys are lost, user can recover with mnemonic
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

pub mod error;
pub mod models;
pub mod repository;
pub mod service;

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
    // Recovery types
    CompleteRecoveryRequest, StartRecoveryRequest,
    // WebAuthn re-exports (for client use)
    CreationChallengeResponse, Passkey, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse,
};
pub use repository::{
    AuthRepository, ChallengeRepository, DeviceRepository, PasskeyRepository, SessionRepository,
    UserRepository,
};
pub use service::{AuthConfig, AuthService};
