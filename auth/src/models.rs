//! Data models for authentication and device management.

use chrono::{DateTime, Utc};
use crypto::{EncryptedBlob, KdfParams};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

impl UserId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub Uuid);

impl DeviceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// User account with encrypted key material.
///
/// The server stores:
/// - Master password hash (for authentication, NOT the password)
/// - Encrypted symmetric key (encrypted with user's stretched key)
/// - KDF parameters (so client can derive the same keys)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user identifier.
    pub id: UserId,

    /// User's email address (used as salt component).
    pub email: String,

    /// Master password hash (base64, for server-side verification).
    /// This is NOT the password - it's derived via PBKDF2 from the master key.
    pub master_password_hash: String,

    /// KDF parameters used to derive the master key.
    pub kdf_params: KdfParams,

    /// User's symmetric key, encrypted with their stretched key.
    /// Only the user can decrypt this.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// Optional encrypted recovery key blob.
    /// If set, user can recover account with BIP39 mnemonic.
    pub encrypted_recovery_key: Option<EncryptedBlob>,

    /// Hash of the recovery verification key (derived from mnemonic).
    /// Used to verify the user knows the mnemonic during recovery.
    /// This is SHA-256(Argon2id(mnemonic, email)).
    pub recovery_verification_hash: Option<String>,

    /// Whether recovery is enabled for this account.
    pub recovery_enabled: bool,

    /// Account creation timestamp.
    pub created_at: DateTime<Utc>,

    /// Last successful login timestamp.
    pub last_login_at: Option<DateTime<Utc>>,

    /// Number of failed login attempts (for rate limiting).
    pub failed_login_attempts: u32,

    /// Account locked until this time (if locked).
    pub locked_until: Option<DateTime<Utc>>,
}

impl User {
    /// Create a new user with the given parameters.
    pub fn new(
        email: String,
        master_password_hash: String,
        kdf_params: KdfParams,
        encrypted_symmetric_key: EncryptedBlob,
    ) -> Self {
        Self {
            id: UserId::new(),
            email,
            master_password_hash,
            kdf_params,
            encrypted_symmetric_key,
            encrypted_recovery_key: None,
            recovery_verification_hash: None,
            recovery_enabled: false,
            created_at: Utc::now(),
            last_login_at: None,
            failed_login_attempts: 0,
            locked_until: None,
        }
    }

    /// Check if the account is currently locked.
    pub fn is_locked(&self) -> bool {
        self.locked_until
            .is_some_and(|locked_until| Utc::now() < locked_until)
    }
}

/// A registered device that can access the user's account.
///
/// Each device stores its own copy of the user's symmetric key,
/// encrypted with device-specific keys derived from the user's password.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Unique device identifier.
    pub id: DeviceId,

    /// User who owns this device.
    pub user_id: UserId,

    /// Human-readable device name (e.g., "Chrome on MacBook").
    pub name: String,

    /// Device type for UI display.
    pub device_type: DeviceType,

    /// User's symmetric key encrypted for this specific device.
    /// Encrypted with a key derived from user's password + device-specific salt.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// Device-specific KDF parameters (may differ from user's main params).
    pub kdf_params: KdfParams,

    /// When this device was registered.
    pub created_at: DateTime<Utc>,

    /// Last time this device was used.
    pub last_used_at: Option<DateTime<Utc>>,

    /// Whether this device is currently active/trusted.
    pub is_active: bool,
}

impl Device {
    /// Create a new device.
    pub fn new(
        user_id: UserId,
        name: String,
        device_type: DeviceType,
        encrypted_symmetric_key: EncryptedBlob,
        kdf_params: KdfParams,
    ) -> Self {
        Self {
            id: DeviceId::new(),
            user_id,
            name,
            device_type,
            encrypted_symmetric_key,
            kdf_params,
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        }
    }
}

/// Type of device for UI categorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    /// Web browser.
    Browser,
    /// Desktop application.
    Desktop,
    /// Mobile application.
    Mobile,
    /// API client (programmatic access).
    ApiClient,
    /// Unknown/other device type.
    Unknown,
}

impl Default for DeviceType {
    fn default() -> Self {
        Self::Unknown
    }
}

/// An active login session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,

    /// User this session belongs to.
    pub user_id: UserId,

    /// Device used for this session.
    pub device_id: DeviceId,

    /// Session creation time.
    pub created_at: DateTime<Utc>,

    /// Session expiration time.
    pub expires_at: DateTime<Utc>,

    /// Last activity time (for idle timeout).
    pub last_activity_at: DateTime<Utc>,

    /// IP address of the client (for audit).
    pub ip_address: Option<String>,

    /// User agent string (for audit/display).
    pub user_agent: Option<String>,
}

impl Session {
    /// Create a new session with default 24-hour expiration.
    pub fn new(user_id: UserId, device_id: DeviceId) -> Self {
        let now = Utc::now();
        Self {
            id: SessionId::new(),
            user_id,
            device_id,
            created_at: now,
            expires_at: now + chrono::Duration::hours(24),
            last_activity_at: now,
            ip_address: None,
            user_agent: None,
        }
    }

    /// Create a session with custom expiration.
    pub fn with_expiration(
        user_id: UserId,
        device_id: DeviceId,
        expires_at: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: SessionId::new(),
            user_id,
            device_id,
            created_at: now,
            expires_at,
            last_activity_at: now,
            ip_address: None,
            user_agent: None,
        }
    }

    /// Check if the session is expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Update last activity timestamp.
    pub fn touch(&mut self) {
        self.last_activity_at = Utc::now();
    }
}

/// Sanitized user information for API responses.
/// Does NOT include sensitive fields like master_password_hash or recovery_verification_hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// Unique user identifier.
    pub id: UserId,

    /// User's email address.
    pub email: String,

    /// Whether recovery is enabled for this account.
    pub recovery_enabled: bool,

    /// Account creation timestamp.
    pub created_at: DateTime<Utc>,

    /// Last successful login timestamp.
    pub last_login_at: Option<DateTime<Utc>>,
}

impl From<&User> for UserInfo {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            recovery_enabled: user.recovery_enabled,
            created_at: user.created_at,
            last_login_at: user.last_login_at,
        }
    }
}

/// Sanitized device information for API responses.
/// Does NOT include the encrypted_symmetric_key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Unique device identifier.
    pub id: DeviceId,

    /// Human-readable device name (e.g., "Chrome on MacBook").
    pub name: String,

    /// Device type for UI display.
    pub device_type: DeviceType,

    /// When this device was registered.
    pub created_at: DateTime<Utc>,

    /// Last time this device was used.
    pub last_used_at: Option<DateTime<Utc>>,

    /// Whether this device is currently active/trusted.
    pub is_active: bool,
}

impl From<&Device> for DeviceInfo {
    fn from(device: &Device) -> Self {
        Self {
            id: device.id,
            name: device.name.clone(),
            device_type: device.device_type,
            created_at: device.created_at,
            last_used_at: device.last_used_at,
            is_active: device.is_active,
        }
    }
}

/// Data returned to client after successful login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    /// Session token for subsequent requests.
    pub session_id: SessionId,

    /// User's encrypted symmetric key (client decrypts with stretched key).
    pub encrypted_symmetric_key: EncryptedBlob,

    /// KDF parameters for deriving the stretched key.
    pub kdf_params: KdfParams,

    /// User's email (for client-side salt).
    pub email: String,

    /// Session expiration time.
    pub expires_at: DateTime<Utc>,
}

/// Data for registering a new user.
///
/// Implements ZeroizeOnDrop to clear sensitive password hash from memory.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct RegisterRequest {
    /// User's email address.
    pub email: String,

    /// Master password hash (derived client-side).
    /// SENSITIVE: Zeroized on drop.
    pub master_password_hash: String,

    /// KDF parameters used by the client.
    #[zeroize(skip)]
    pub kdf_params: KdfParams,

    /// User's symmetric key, encrypted with stretched key.
    #[zeroize(skip)]
    pub encrypted_symmetric_key: EncryptedBlob,

    /// Device name for the first device.
    pub device_name: String,

    /// Device type.
    #[zeroize(skip)]
    pub device_type: DeviceType,

    /// Optional: encrypted recovery key if user wants BIP39 backup.
    #[zeroize(skip)]
    pub encrypted_recovery_key: Option<EncryptedBlob>,

    /// Optional: hash of recovery verification key (required if encrypted_recovery_key is set).
    /// Client derives this as: base64(SHA-256(Argon2id(mnemonic, email))).
    /// SENSITIVE: Zeroized on drop.
    pub recovery_verification_hash: Option<String>,
}

/// Data for logging in.
///
/// Implements ZeroizeOnDrop to clear sensitive password hash from memory.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct LoginRequest {
    /// User's email address.
    pub email: String,

    /// Master password hash (derived client-side).
    /// SENSITIVE: Zeroized on drop.
    pub master_password_hash: String,

    /// Device name (for new device registration or identification).
    pub device_name: String,

    /// Device type.
    #[zeroize(skip)]
    pub device_type: DeviceType,
}

/// Data for account recovery using BIP39 mnemonic.
///
/// Implements ZeroizeOnDrop to clear sensitive hashes from memory.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct RecoveryRequest {
    /// User's email address.
    pub email: String,

    /// Recovery verification hash to prove possession of mnemonic.
    /// Client derives this as: base64(SHA-256(Argon2id(mnemonic, email))).
    /// Must match the hash stored during registration.
    /// SENSITIVE: Zeroized on drop.
    pub recovery_verification_hash: String,

    /// New master password hash (after recovery).
    /// SENSITIVE: Zeroized on drop.
    pub new_master_password_hash: String,

    /// New KDF parameters.
    #[zeroize(skip)]
    pub new_kdf_params: KdfParams,

    /// New encrypted symmetric key (re-encrypted with new password).
    #[zeroize(skip)]
    pub new_encrypted_symmetric_key: EncryptedBlob,

    /// Optional: new recovery verification hash if user wants to keep recovery enabled.
    /// If not provided, recovery will be disabled after account recovery.
    /// SENSITIVE: Zeroized on drop.
    pub new_recovery_verification_hash: Option<String>,

    /// Optional: new encrypted recovery key if user wants to keep recovery enabled.
    #[zeroize(skip)]
    pub new_encrypted_recovery_key: Option<EncryptedBlob>,
}

// ============================================================================
// Passkey/WebAuthn Models
// ============================================================================

/// Unique identifier for a passkey credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PasskeyId(pub Uuid);

impl PasskeyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PasskeyId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PasskeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A stored passkey credential for WebAuthn authentication.
///
/// This wraps the webauthn-rs `Passkey` type with additional metadata.
/// Passkeys are the RECOMMENDED authentication method (over email+password).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyCredential {
    /// Unique identifier for this credential.
    pub id: PasskeyId,

    /// User who owns this passkey.
    pub user_id: UserId,

    /// Human-readable name for this passkey (e.g., "MacBook Pro Touch ID").
    pub name: String,

    /// The actual WebAuthn passkey data (credential ID, public key, etc.).
    /// This is the serialized webauthn-rs Passkey type.
    pub passkey: Passkey,

    /// When this passkey was registered.
    pub created_at: DateTime<Utc>,

    /// Last time this passkey was used for authentication.
    pub last_used_at: Option<DateTime<Utc>>,

    /// Whether this passkey is currently active.
    pub is_active: bool,
}

impl PasskeyCredential {
    /// Create a new passkey credential.
    pub fn new(user_id: UserId, name: String, passkey: Passkey) -> Self {
        Self {
            id: PasskeyId::new(),
            user_id,
            name,
            passkey,
            created_at: Utc::now(),
            last_used_at: None,
            is_active: true,
        }
    }
}

/// Sanitized passkey information for API responses.
/// Does NOT include the actual passkey data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyInfo {
    /// Unique passkey identifier.
    pub id: PasskeyId,

    /// Human-readable passkey name.
    pub name: String,

    /// When this passkey was registered.
    pub created_at: DateTime<Utc>,

    /// Last time this passkey was used.
    pub last_used_at: Option<DateTime<Utc>>,

    /// Whether this passkey is currently active.
    pub is_active: bool,
}

impl From<&PasskeyCredential> for PasskeyInfo {
    fn from(cred: &PasskeyCredential) -> Self {
        Self {
            id: cred.id,
            name: cred.name.clone(),
            created_at: cred.created_at,
            last_used_at: cred.last_used_at,
            is_active: cred.is_active,
        }
    }
}

/// Request to start passkey registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartPasskeyRegistrationRequest {
    /// Human-readable name for the passkey (e.g., "MacBook Pro Touch ID").
    pub passkey_name: String,
}

/// Response for starting passkey registration.
/// Client uses this to prompt the user's authenticator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartPasskeyRegistrationResponse {
    /// WebAuthn credential creation options for the client.
    pub options: CreationChallengeResponse,
}

/// Request to complete passkey registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletePasskeyRegistrationRequest {
    /// The passkey name provided in the start request.
    pub passkey_name: String,

    /// The credential response from the authenticator.
    pub credential: RegisterPublicKeyCredential,
}

/// Response for starting passkey authentication.
/// Client uses this to prompt the user's authenticator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartPasskeyLoginResponse {
    /// WebAuthn request options for the client.
    pub options: RequestChallengeResponse,
}

/// Request to complete passkey authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletePasskeyLoginRequest {
    /// User's email (to look up their passkeys).
    pub email: String,

    /// The credential response from the authenticator.
    pub credential: PublicKeyCredential,

    /// Device name for the session.
    pub device_name: String,

    /// Device type.
    pub device_type: DeviceType,
}

/// Data for registering a new user with passkey (recommended flow).
///
/// Unlike password registration, this doesn't require a master password hash.
/// The user's symmetric key is encrypted with a key derived from the passkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyRegisterRequest {
    /// User's email address.
    pub email: String,

    /// KDF parameters used by the client.
    pub kdf_params: KdfParams,

    /// User's symmetric key, encrypted with a key derived from passkey material.
    /// Note: The client must implement key derivation from passkey output.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// Device name for the first device.
    pub device_name: String,

    /// Device type.
    pub device_type: DeviceType,

    /// Human-readable name for the passkey.
    pub passkey_name: String,

    /// Optional: encrypted recovery key if user wants BIP39 backup.
    pub encrypted_recovery_key: Option<EncryptedBlob>,

    /// Optional: hash of recovery verification key.
    pub recovery_verification_hash: Option<String>,
}
