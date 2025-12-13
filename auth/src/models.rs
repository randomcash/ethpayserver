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
/// Authentication is passkey-only. The server stores:
/// - Encrypted symmetric key (encrypted with mnemonic-derived key)
/// - KDF parameters (so client can derive the same keys)
/// - Recovery verification hash (to verify mnemonic during recovery)
///
/// Passkeys are stored separately in PasskeyCredential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user identifier.
    pub id: UserId,

    /// User's email address (used as salt component for key derivation).
    pub email: String,

    /// KDF parameters used to derive keys from the mnemonic.
    pub kdf_params: KdfParams,

    /// User's symmetric key, encrypted with mnemonic-derived key.
    /// Only the user (with the mnemonic) can decrypt this.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// Hash of the recovery verification key (derived from mnemonic).
    /// Used to verify the user knows the mnemonic during recovery.
    /// This is base64(SHA-256(Argon2id(mnemonic, email))).
    /// Required for account recovery.
    pub recovery_verification_hash: String,

    /// Account creation timestamp.
    pub created_at: DateTime<Utc>,

    /// Last successful login timestamp.
    pub last_login_at: Option<DateTime<Utc>>,

    /// Number of failed login/recovery attempts (for rate limiting).
    pub failed_login_attempts: u32,

    /// Account locked until this time (if locked).
    pub locked_until: Option<DateTime<Utc>>,
}

impl User {
    /// Create a new user with the given parameters.
    ///
    /// The recovery_verification_hash is required as it's needed for account recovery.
    pub fn new(
        email: String,
        kdf_params: KdfParams,
        encrypted_symmetric_key: EncryptedBlob,
        recovery_verification_hash: String,
    ) -> Self {
        Self {
            id: UserId::new(),
            email,
            kdf_params,
            encrypted_symmetric_key,
            recovery_verification_hash,
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
/// Each device stores the user's encrypted symmetric key for offline access.
/// The key is encrypted with a key derived from the user's BIP39 mnemonic.
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

    /// User's symmetric key encrypted for this device.
    /// Encrypted with a key derived from the user's mnemonic.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// KDF parameters used to derive the encryption key.
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
/// Does NOT include sensitive fields like recovery_verification_hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// Unique user identifier.
    pub id: UserId,

    /// User's email address.
    pub email: String,

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

/// Data returned to client after successful login or registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    /// Session token for subsequent requests.
    pub session_id: SessionId,

    /// Device ID for this device.
    /// Client should store this locally and send it in future login requests
    /// to identify the same device.
    pub device_id: DeviceId,

    /// User's encrypted symmetric key.
    /// Client decrypts with mnemonic-derived key.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// KDF parameters for deriving the decryption key from mnemonic.
    pub kdf_params: KdfParams,

    /// User's email (used as salt for key derivation).
    pub email: String,

    /// Session expiration time.
    pub expires_at: DateTime<Utc>,
}

// Note: Password-based RegisterRequest and LoginRequest have been removed.
// Use passkey authentication instead:
// - New users: start_new_user_passkey_registration + complete_new_user_passkey_registration
// - Existing users: start_passkey_login + complete_passkey_login

/// Request to start account recovery using BIP39 mnemonic.
///
/// This is the first step of the recovery process. After verification,
/// the server returns a passkey registration challenge.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct StartRecoveryRequest {
    /// User's email address.
    pub email: String,

    /// Recovery verification hash to prove possession of mnemonic.
    /// Client derives this as: base64(SHA-256(Argon2id(mnemonic, email))).
    /// Must match the hash stored during registration.
    /// SENSITIVE: Zeroized on drop.
    pub recovery_verification_hash: String,
}

/// Request to complete account recovery.
///
/// After verifying the mnemonic and registering a new passkey,
/// this completes the recovery process.
///
/// The `new_recovery_verification_hash` field is zeroized on drop for security.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteRecoveryRequest {
    /// The passkey credential from the authenticator.
    pub credential: RegisterPublicKeyCredential,

    /// Human-readable name for the new passkey.
    pub passkey_name: String,

    /// New KDF parameters.
    pub new_kdf_params: KdfParams,

    /// New encrypted symmetric key (re-encrypted with recovery-derived key).
    pub new_encrypted_symmetric_key: EncryptedBlob,

    /// New recovery verification hash.
    /// Required because the hash depends on KDF params.
    /// Client derives: base64(SHA-256(Argon2id(mnemonic, email, new_kdf_params))).
    /// SENSITIVE: Zeroized on drop.
    pub new_recovery_verification_hash: String,

    /// Device name for the recovery device.
    pub device_name: String,

    /// Device type.
    pub device_type: DeviceType,
}

impl Zeroize for CompleteRecoveryRequest {
    fn zeroize(&mut self) {
        self.new_recovery_verification_hash.zeroize();
    }
}

impl Drop for CompleteRecoveryRequest {
    fn drop(&mut self) {
        self.zeroize();
    }
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
/// Passkeys are the primary authentication method for this system.
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

/// Response for starting NEW USER passkey registration.
/// Includes the temporary user ID needed to complete registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartNewUserPasskeyRegistrationResponse {
    /// WebAuthn credential creation options for the client.
    pub options: CreationChallengeResponse,

    /// Temporary user ID. Must be passed back to complete_new_user_passkey_registration.
    /// This is the user ID that will be assigned to the new user.
    pub user_id: UserId,

    /// The email address (normalized to lowercase).
    /// Must match the email in CompleteNewUserPasskeyRegistrationRequest.
    pub email: String,
}

/// Request to complete NEW USER passkey registration.
/// Combines the passkey credential with user account details.
///
/// The `recovery_verification_hash` field is zeroized on drop for security.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteNewUserPasskeyRegistrationRequest {
    /// The temporary user ID from StartNewUserPasskeyRegistrationResponse.
    pub user_id: UserId,

    /// The credential response from the authenticator.
    pub credential: RegisterPublicKeyCredential,

    /// User's email address.
    pub email: String,

    /// KDF parameters used to derive keys from mnemonic.
    pub kdf_params: KdfParams,

    /// User's symmetric key, encrypted with mnemonic-derived key.
    pub encrypted_symmetric_key: EncryptedBlob,

    /// Hash of the recovery verification key (derived from mnemonic).
    /// REQUIRED for account recovery.
    /// Client derives: base64(SHA-256(Argon2id(mnemonic, email))).
    /// SENSITIVE: Zeroized on drop.
    pub recovery_verification_hash: String,

    /// Device name for the first device.
    pub device_name: String,

    /// Device type (browser, mobile, etc.).
    pub device_type: DeviceType,

    /// Name for the passkey (e.g., "MacBook Pro Touch ID").
    pub passkey_name: String,
}

impl Zeroize for CompleteNewUserPasskeyRegistrationRequest {
    fn zeroize(&mut self) {
        self.recovery_verification_hash.zeroize();
    }
}

impl Drop for CompleteNewUserPasskeyRegistrationRequest {
    fn drop(&mut self) {
        self.zeroize();
    }
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

    /// Device ID from a previous login on this device.
    /// If provided, the server will reuse the existing device record.
    /// If None, a new device will be created.
    /// Client should store the device_id from LoginResponse and send it here.
    pub device_id: Option<DeviceId>,

    /// Device name for the session (used when creating a new device).
    pub device_name: String,

    /// Device type (used when creating a new device).
    pub device_type: DeviceType,
}

// Note: PasskeyRegisterRequest has been replaced by CompleteNewUserPasskeyRegistrationRequest
// which includes the user_id and credential from the WebAuthn registration flow.
