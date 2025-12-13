//! User authentication and device management for PayServer.
//!
//! This crate provides zero-knowledge authentication following the Bitwarden model:
//! - Server never sees plaintext passwords or keys
//! - All sensitive data is encrypted client-side
//! - Server stores only encrypted blobs it cannot decrypt
//!
//! # Architecture
//!
//! ```text
//! Client                              Server
//! ──────                              ──────
//! Password + Email
//!       │
//!       ▼ Argon2id
//! Master Key
//!       │
//!   ┌───┴───┐
//!   │       │ HKDF
//!   ▼       ▼
//! Stretched Key   Master Password Hash ───► Stored (verification)
//!       │
//!       ▼ Encrypt
//! Encrypted Symmetric Key ────────────────► Stored (user can decrypt)
//! ```
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
//! // Register a new user
//! let response = service.register(request).await?;
//!
//! // Login
//! let response = service.login(login_request).await?;
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
    Device, DeviceId, DeviceInfo, DeviceType, LoginRequest, LoginResponse, RecoveryRequest,
    RegisterRequest, Session, SessionId, User, UserId, UserInfo,
};
pub use repository::{AuthRepository, DeviceRepository, SessionRepository, UserRepository};
pub use service::{AuthConfig, AuthService};
