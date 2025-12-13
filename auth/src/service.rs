//! Authentication service with business logic.

use std::sync::Arc;

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::{AuthError, Result};
use crate::models::{
    Device, DeviceId, DeviceInfo, DeviceType, LoginRequest, LoginResponse, RecoveryRequest,
    RegisterRequest, Session, SessionId, User, UserInfo,
};
use crate::repository::{DeviceRepository, SessionRepository, UserRepository};

/// Validates email format.
/// Checks for basic structure: non-empty local part, @, non-empty domain with at least one dot.
fn validate_email(email: &str) -> Result<()> {
    let email = email.trim();

    if email.is_empty() {
        return Err(AuthError::InvalidEmail("email cannot be empty".into()));
    }

    // Split at @ and validate parts
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return Err(AuthError::InvalidEmail("email must contain exactly one @".into()));
    }

    let local = parts[0];
    let domain = parts[1];

    if local.is_empty() {
        return Err(AuthError::InvalidEmail("local part cannot be empty".into()));
    }

    if domain.is_empty() {
        return Err(AuthError::InvalidEmail("domain cannot be empty".into()));
    }

    // Domain must have at least one dot (e.g., example.com)
    if !domain.contains('.') {
        return Err(AuthError::InvalidEmail("domain must contain a dot".into()));
    }

    // Domain cannot start or end with a dot
    if domain.starts_with('.') || domain.ends_with('.') {
        return Err(AuthError::InvalidEmail("domain cannot start or end with a dot".into()));
    }

    Ok(())
}

/// Configuration for the auth service.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Maximum failed login attempts before lockout.
    pub max_failed_attempts: u32,

    /// Lockout duration after max failed attempts.
    pub lockout_duration: Duration,

    /// Session expiration time (absolute timeout).
    pub session_duration: Duration,

    /// Session idle timeout. If None, idle timeout is disabled.
    /// Session expires if no activity for this duration.
    pub idle_timeout: Option<Duration>,

    /// Maximum devices per user.
    pub max_devices_per_user: u32,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            max_failed_attempts: 5,
            lockout_duration: Duration::minutes(15),
            session_duration: Duration::hours(24),
            idle_timeout: Some(Duration::hours(2)), // 2 hour idle timeout
            max_devices_per_user: 10,
        }
    }
}

/// Authentication service handling registration, login, and session management.
pub struct AuthService<R>
where
    R: UserRepository + DeviceRepository + SessionRepository,
{
    repo: Arc<R>,
    config: AuthConfig,
}

impl<R> AuthService<R>
where
    R: UserRepository + DeviceRepository + SessionRepository,
{
    /// Create a new auth service with the given repository and default config.
    pub fn new(repo: Arc<R>) -> Self {
        Self {
            repo,
            config: AuthConfig::default(),
        }
    }

    /// Create a new auth service with custom config.
    pub fn with_config(repo: Arc<R>, config: AuthConfig) -> Self {
        Self { repo, config }
    }

    /// Register a new user.
    ///
    /// The client must:
    /// 1. Derive master key from password + email using Argon2id
    /// 2. Stretch master key using HKDF
    /// 3. Generate random symmetric key
    /// 4. Encrypt symmetric key with stretched key
    /// 5. Derive master password hash (PBKDF2 of master key + password)
    /// 6. If enabling recovery:
    ///    a. Generate BIP39 mnemonic
    ///    b. Derive recovery key from mnemonic + email using Argon2id
    ///    c. Encrypt symmetric key with recovery key
    ///    d. Hash recovery key: base64(SHA-256(recovery_key))
    /// 7. Send all encrypted data to server
    pub async fn register(&self, request: RegisterRequest) -> Result<LoginResponse> {
        // Validate email format
        validate_email(&request.email)?;

        // Check if user already exists
        let email_lower = request.email.to_lowercase();
        if self.repo.get_user_by_email(&email_lower).await?.is_some() {
            return Err(AuthError::UserExists(email_lower));
        }

        // Validate recovery setup: both encrypted key and verification hash must be present together
        if request.encrypted_recovery_key.is_some() != request.recovery_verification_hash.is_some() {
            return Err(AuthError::InvalidRecoverySetup);
        }

        // Create user
        let user = User::new(
            email_lower.clone(),
            request.master_password_hash.clone(),
            request.kdf_params.clone(),
            request.encrypted_symmetric_key.clone(),
        );
        let user_id = user.id;

        // Set up recovery if provided
        let mut user = user;
        if let (Some(encrypted_key), Some(verification_hash)) =
            (request.encrypted_recovery_key.clone(), request.recovery_verification_hash.clone())
        {
            user.encrypted_recovery_key = Some(encrypted_key);
            user.recovery_verification_hash = Some(verification_hash);
            user.recovery_enabled = true;
        }

        self.repo.create_user(&user).await?;

        // Create first device
        let device = Device::new(
            user_id,
            request.device_name.clone(),
            request.device_type,
            request.encrypted_symmetric_key.clone(),
            request.kdf_params.clone(),
        );
        let device_id = device.id;
        self.repo.create_device(&device).await?;

        // Create session
        let expires_at = Utc::now() + self.config.session_duration;
        let session = Session::with_expiration(user_id, device_id, expires_at);
        let session_id = session.id;
        self.repo.create_session(&session).await?;

        Ok(LoginResponse {
            session_id,
            encrypted_symmetric_key: request.encrypted_symmetric_key.clone(),
            kdf_params: request.kdf_params.clone(),
            email: email_lower,
            expires_at,
        })
    }

    /// Authenticate a user and create a session.
    ///
    /// Returns the encrypted symmetric key and KDF params so the client
    /// can derive keys and decrypt user data.
    pub async fn login(&self, request: LoginRequest) -> Result<LoginResponse> {
        let email_lower = request.email.to_lowercase();

        // Find user
        let user = self
            .repo
            .get_user_by_email(&email_lower)
            .await?
            .ok_or_else(|| AuthError::InvalidCredentials)?;

        // Verify master password hash using constant-time comparison.
        // We hash both inputs with SHA-256 first to ensure fixed-length comparison,
        // preventing timing attacks based on input length differences.
        let password_matches = {
            let stored_hash = Sha256::digest(user.master_password_hash.as_bytes());
            let provided_hash = Sha256::digest(request.master_password_hash.as_bytes());
            stored_hash.ct_eq(&provided_hash).unwrap_u8() == 1
        };

        if !password_matches {
            // Increment failed attempts (don't reveal if account is locked)
            let attempts = self.repo.increment_failed_logins(user.id).await?;

            // Lock account if too many failures (silently)
            if attempts >= self.config.max_failed_attempts {
                let lock_until = Utc::now() + self.config.lockout_duration;
                self.repo.lock_user(user.id, lock_until).await?;
            }

            // Always return InvalidCredentials to prevent user enumeration
            return Err(AuthError::InvalidCredentials);
        }

        // Password is correct - NOW check if account is locked
        // This prevents revealing account existence via lockout errors
        if user.is_locked() {
            return Err(AuthError::AccountLocked);
        }

        // Reset failed attempts on successful login
        self.repo.reset_failed_logins(user.id).await?;

        // Find or create device
        let devices = self.repo.get_devices_for_user(user.id).await?;
        let device = devices
            .iter()
            .find(|d| d.name == request.device_name && d.is_active);

        let device_id = if let Some(existing) = device {
            // Update last used
            let mut updated = existing.clone();
            updated.last_used_at = Some(Utc::now());
            self.repo.update_device(&updated).await?;
            existing.id
        } else {
            // Check device limit
            let active_count = self.repo.count_active_devices(user.id).await?;
            if active_count >= self.config.max_devices_per_user {
                return Err(AuthError::MaxDevicesReached(self.config.max_devices_per_user));
            }

            // Create new device
            let new_device = Device::new(
                user.id,
                request.device_name.clone(),
                request.device_type,
                user.encrypted_symmetric_key.clone(),
                user.kdf_params.clone(),
            );
            let id = new_device.id;
            self.repo.create_device(&new_device).await?;
            id
        };

        // Update last login
        let mut updated_user = user.clone();
        updated_user.last_login_at = Some(Utc::now());
        self.repo.update_user(&updated_user).await?;

        // Create session
        let expires_at = Utc::now() + self.config.session_duration;
        let session = Session::with_expiration(user.id, device_id, expires_at);
        let session_id = session.id;
        self.repo.create_session(&session).await?;

        Ok(LoginResponse {
            session_id,
            encrypted_symmetric_key: user.encrypted_symmetric_key,
            kdf_params: user.kdf_params,
            email: user.email,
            expires_at,
        })
    }

    /// Validate a session and return sanitized user info.
    ///
    /// Checks both absolute expiration and idle timeout (if configured).
    /// Returns UserInfo (without sensitive fields) instead of full User.
    pub async fn validate_session(&self, session_id: SessionId) -> Result<(UserInfo, Session)> {
        let session = self
            .repo
            .get_session(session_id)
            .await?
            .ok_or(AuthError::SessionInvalid)?;

        // Check absolute expiration
        if session.is_expired() {
            self.repo.delete_session(session_id).await?;
            return Err(AuthError::SessionInvalid);
        }

        // Check idle timeout if configured
        if let Some(idle_timeout) = self.config.idle_timeout {
            let idle_deadline = session.last_activity_at + idle_timeout;
            if Utc::now() > idle_deadline {
                self.repo.delete_session(session_id).await?;
                return Err(AuthError::SessionInvalid);
            }
        }

        let user = self
            .repo
            .get_user(session.user_id)
            .await?
            .ok_or(AuthError::SessionInvalid)?;

        // Update session activity
        let mut updated_session = session.clone();
        updated_session.touch();
        self.repo.update_session(&updated_session).await?;

        Ok((UserInfo::from(&user), updated_session))
    }

    /// Logout - invalidate a session.
    pub async fn logout(&self, session_id: SessionId) -> Result<()> {
        self.repo.delete_session(session_id).await
    }

    /// Logout from all devices.
    ///
    /// Requires a valid session to prove ownership. The provided session
    /// will also be invalidated along with all other sessions.
    pub async fn logout_all(&self, session_id: SessionId) -> Result<()> {
        // Validate the caller's session to get user_id
        let session = self
            .repo
            .get_session(session_id)
            .await?
            .ok_or(AuthError::SessionInvalid)?;

        // Check absolute expiration
        if session.is_expired() {
            self.repo.delete_session(session_id).await?;
            return Err(AuthError::SessionInvalid);
        }

        // Check idle timeout if configured
        if let Some(idle_timeout) = self.config.idle_timeout {
            let idle_deadline = session.last_activity_at + idle_timeout;
            if Utc::now() > idle_deadline {
                self.repo.delete_session(session_id).await?;
                return Err(AuthError::SessionInvalid);
            }
        }

        // Delete all sessions for this user (including the current one)
        self.repo.delete_all_sessions_for_user(session.user_id).await
    }

    /// Get all active devices for a user.
    ///
    /// Requires a valid session for authentication.
    /// Returns sanitized DeviceInfo (without encrypted keys) for active devices only.
    pub async fn get_devices(&self, session_id: SessionId) -> Result<Vec<DeviceInfo>> {
        // Validate session to authenticate the request
        let session = self
            .repo
            .get_session(session_id)
            .await?
            .ok_or(AuthError::SessionInvalid)?;

        // Check absolute expiration
        if session.is_expired() {
            self.repo.delete_session(session_id).await?;
            return Err(AuthError::SessionInvalid);
        }

        // Check idle timeout if configured
        if let Some(idle_timeout) = self.config.idle_timeout {
            let idle_deadline = session.last_activity_at + idle_timeout;
            if Utc::now() > idle_deadline {
                self.repo.delete_session(session_id).await?;
                return Err(AuthError::SessionInvalid);
            }
        }

        let devices = self.repo.get_devices_for_user(session.user_id).await?;

        // Filter to active devices only and convert to DeviceInfo
        Ok(devices
            .iter()
            .filter(|d| d.is_active)
            .map(DeviceInfo::from)
            .collect())
    }

    /// Revoke a device (removes its encrypted key, invalidates sessions).
    ///
    /// Requires a valid session for authentication.
    /// Cannot revoke the device associated with the current session - use logout instead.
    pub async fn revoke_device(&self, session_id: SessionId, device_id: DeviceId) -> Result<()> {
        // Validate session to authenticate the request
        let session = self
            .repo
            .get_session(session_id)
            .await?
            .ok_or(AuthError::SessionInvalid)?;

        // Check absolute expiration
        if session.is_expired() {
            self.repo.delete_session(session_id).await?;
            return Err(AuthError::SessionInvalid);
        }

        // Check idle timeout if configured
        if let Some(idle_timeout) = self.config.idle_timeout {
            let idle_deadline = session.last_activity_at + idle_timeout;
            if Utc::now() > idle_deadline {
                self.repo.delete_session(session_id).await?;
                return Err(AuthError::SessionInvalid);
            }
        }

        // Cannot revoke the device you're currently using
        if session.device_id == device_id {
            return Err(AuthError::CannotRevokeCurrentDevice);
        }

        let user_id = session.user_id;

        // Verify the device belongs to this user
        let device = self
            .repo
            .get_device(device_id)
            .await?
            .ok_or(AuthError::DeviceNotFound(device_id.to_string()))?;

        if device.user_id != user_id {
            return Err(AuthError::DeviceNotFound(device_id.to_string()));
        }

        // Delete all sessions for this device
        self.repo.delete_sessions_for_device(device_id).await?;

        // Deactivate the device
        self.repo.deactivate_device(device_id).await
    }

    /// Recover account using BIP39 mnemonic.
    ///
    /// The client must:
    /// 1. Derive recovery key from mnemonic + email using Argon2id
    /// 2. Hash recovery key: base64(SHA-256(recovery_key)) for verification
    /// 3. Decrypt the stored encrypted_recovery_key to get the symmetric key
    /// 4. Re-encrypt symmetric key with new password's stretched key
    /// 5. Derive new master password hash
    /// 6. Send recovery_verification_hash + new encrypted data to server
    ///
    /// This revokes all existing devices (they have old password's encryption).
    ///
    /// # Rate Limiting
    /// Failed recovery attempts are tracked and the account is locked after
    /// too many failures (same as login).
    pub async fn recover_account(&self, request: RecoveryRequest) -> Result<LoginResponse> {
        let email_lower = request.email.to_lowercase();

        // Find user - return generic error to prevent user enumeration
        let user = match self.repo.get_user_by_email(&email_lower).await? {
            Some(u) => u,
            None => return Err(AuthError::InvalidRecoveryMnemonic),
        };

        // Verify recovery is enabled and we have a stored verification hash
        // Return generic error to not reveal recovery status
        if !user.recovery_enabled || user.recovery_verification_hash.is_none() {
            return Err(AuthError::InvalidRecoveryMnemonic);
        }

        // CRITICAL: Verify the client knows the recovery mnemonic by comparing hashes.
        // The client derives: base64(SHA-256(Argon2id(mnemonic, email)))
        // We compare this with the stored hash using constant-time comparison.
        let stored_hash = user.recovery_verification_hash.as_ref().unwrap();
        let verification_matches = {
            let stored = Sha256::digest(stored_hash.as_bytes());
            let provided = Sha256::digest(request.recovery_verification_hash.as_bytes());
            stored.ct_eq(&provided).unwrap_u8() == 1
        };

        if !verification_matches {
            // Track failed recovery attempts using the same counter as login
            let attempts = self.repo.increment_failed_logins(user.id).await?;

            // Lock account if too many failures
            if attempts >= self.config.max_failed_attempts {
                let lock_until = Utc::now() + self.config.lockout_duration;
                self.repo.lock_user(user.id, lock_until).await?;
            }

            // Always return InvalidRecoveryMnemonic to prevent user enumeration
            return Err(AuthError::InvalidRecoveryMnemonic);
        }

        // Recovery hash is correct - NOW check if account is locked
        // This prevents revealing account lockout status to attackers
        if user.is_locked() {
            return Err(AuthError::AccountLocked);
        }

        // Reset failed attempts on successful recovery
        self.repo.reset_failed_logins(user.id).await?;

        // Verification passed - proceed with account recovery

        // Revoke all existing devices (they're encrypted with old password)
        self.repo.delete_all_devices_for_user(user.id).await?;

        // Revoke all existing sessions
        self.repo.delete_all_sessions_for_user(user.id).await?;

        // Update user with new credentials
        let mut updated_user = user.clone();
        updated_user.master_password_hash = request.new_master_password_hash.clone();
        updated_user.kdf_params = request.new_kdf_params.clone();
        updated_user.encrypted_symmetric_key = request.new_encrypted_symmetric_key.clone();
        updated_user.failed_login_attempts = 0;
        updated_user.locked_until = None;

        // Handle new recovery setup (optional)
        if let (Some(new_recovery_key), Some(new_verification_hash)) = (
            request.new_encrypted_recovery_key.clone(),
            request.new_recovery_verification_hash.clone(),
        ) {
            updated_user.encrypted_recovery_key = Some(new_recovery_key);
            updated_user.recovery_verification_hash = Some(new_verification_hash);
            updated_user.recovery_enabled = true;
        } else {
            // Disable recovery if new mnemonic not provided
            updated_user.encrypted_recovery_key = None;
            updated_user.recovery_verification_hash = None;
            updated_user.recovery_enabled = false;
        }

        self.repo.update_user(&updated_user).await?;

        // Create new device for recovery session
        let device = Device::new(
            user.id,
            "Recovery Device".to_string(),
            DeviceType::Browser,
            request.new_encrypted_symmetric_key.clone(),
            request.new_kdf_params.clone(),
        );
        let device_id = device.id;
        self.repo.create_device(&device).await?;

        // Create new session
        let expires_at = Utc::now() + self.config.session_duration;
        let session = Session::with_expiration(user.id, device_id, expires_at);
        let session_id = session.id;
        self.repo.create_session(&session).await?;

        Ok(LoginResponse {
            session_id,
            encrypted_symmetric_key: request.new_encrypted_symmetric_key.clone(),
            kdf_params: request.new_kdf_params.clone(),
            email: email_lower,
            expires_at,
        })
    }

    /// Rotate the user's symmetric key (password change).
    ///
    /// This is called when user wants to change their password.
    /// Requires both a valid session AND the current password for security.
    /// All active devices will be re-encrypted with the new key and all sessions invalidated.
    pub async fn rotate_key(
        &self,
        session_id: SessionId,
        old_master_password_hash: String,
        new_master_password_hash: String,
        new_kdf_params: crypto::KdfParams,
        new_encrypted_symmetric_key: crypto::EncryptedBlob,
    ) -> Result<()> {
        // Validate session to authenticate the request
        let session = self
            .repo
            .get_session(session_id)
            .await?
            .ok_or(AuthError::SessionInvalid)?;

        // Check absolute expiration
        if session.is_expired() {
            self.repo.delete_session(session_id).await?;
            return Err(AuthError::SessionInvalid);
        }

        // Check idle timeout if configured
        if let Some(idle_timeout) = self.config.idle_timeout {
            let idle_deadline = session.last_activity_at + idle_timeout;
            if Utc::now() > idle_deadline {
                self.repo.delete_session(session_id).await?;
                return Err(AuthError::SessionInvalid);
            }
        }

        let user_id = session.user_id;
        let user = self
            .repo
            .get_user(user_id)
            .await?
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_string()))?;

        // CRITICAL: Verify old password before allowing password change.
        // This prevents account takeover if only the session is compromised.
        let password_matches = {
            let stored_hash = Sha256::digest(user.master_password_hash.as_bytes());
            let provided_hash = Sha256::digest(old_master_password_hash.as_bytes());
            stored_hash.ct_eq(&provided_hash).unwrap_u8() == 1
        };

        if !password_matches {
            return Err(AuthError::InvalidCredentials);
        }

        // Update user credentials
        let mut updated_user = user;
        updated_user.master_password_hash = new_master_password_hash;
        updated_user.kdf_params = new_kdf_params.clone();
        updated_user.encrypted_symmetric_key = new_encrypted_symmetric_key.clone();
        self.repo.update_user(&updated_user).await?;

        // Update only active devices with new encrypted key (skip deactivated)
        let devices = self.repo.get_devices_for_user(user_id).await?;
        for device in devices {
            if !device.is_active {
                continue; // Skip deactivated devices
            }
            let mut updated_device = device;
            updated_device.encrypted_symmetric_key = new_encrypted_symmetric_key.clone();
            updated_device.kdf_params = new_kdf_params.clone();
            self.repo.update_device(&updated_device).await?;
        }

        // Revoke all sessions (user needs to re-login with new password)
        self.repo.delete_all_sessions_for_user(user_id).await?;

        Ok(())
    }

    /// Clean up stale sessions (call periodically).
    ///
    /// Removes sessions that are either:
    /// - Absolutely expired (past expires_at)
    /// - Idle-timed-out (no activity for idle_timeout duration)
    pub async fn cleanup_stale_sessions(&self) -> Result<u64> {
        self.repo.delete_stale_sessions(self.config.idle_timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::inmemory::InMemoryRepository;
    use crypto::{EncryptedBlob, KdfParams};

    fn test_kdf_params() -> KdfParams {
        KdfParams {
            algorithm: "argon2id".to_string(),
            memory_kb: 65536,
            iterations: 3,
            parallelism: 4,
            salt: vec![0u8; 16],
        }
    }

    fn test_encrypted_blob() -> EncryptedBlob {
        EncryptedBlob::new(vec![1, 2, 3], vec![4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19], vec![0u8; 32])
    }

    fn create_service() -> AuthService<InMemoryRepository> {
        let repo = Arc::new(InMemoryRepository::new());
        AuthService::new(repo)
    }

    fn register_request(email: &str) -> RegisterRequest {
        RegisterRequest {
            email: email.to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            kdf_params: test_kdf_params(),
            encrypted_symmetric_key: test_encrypted_blob(),
            device_name: "Test Browser".to_string(),
            device_type: DeviceType::Browser,
            encrypted_recovery_key: None,
            recovery_verification_hash: None,
        }
    }

    #[tokio::test]
    async fn test_register_success() {
        let service = create_service();
        let request = register_request("test@example.com");

        let response = service.register(request).await.unwrap();

        assert_eq!(response.email, "test@example.com");
        assert!(!response.session_id.0.is_nil());
    }

    #[tokio::test]
    async fn test_register_duplicate_email() {
        let service = create_service();
        let request = register_request("test@example.com");

        service.register(request.clone()).await.unwrap();
        let result = service.register(request).await;

        assert!(matches!(result, Err(AuthError::UserExists(_))));
    }

    #[tokio::test]
    async fn test_register_email_case_insensitive() {
        let service = create_service();

        service
            .register(register_request("Test@Example.COM"))
            .await
            .unwrap();

        let result = service.register(register_request("test@example.com")).await;
        assert!(matches!(result, Err(AuthError::UserExists(_))));
    }

    #[tokio::test]
    async fn test_login_success() {
        let service = create_service();
        let request = register_request("test@example.com");
        service.register(request).await.unwrap();

        let login_request = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            device_name: "Test Browser".to_string(),
            device_type: DeviceType::Browser,
        };

        let response = service.login(login_request).await.unwrap();

        assert_eq!(response.email, "test@example.com");
        assert!(!response.session_id.0.is_nil());
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let service = create_service();
        let request = register_request("test@example.com");
        service.register(request).await.unwrap();

        let login_request = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "wrong_hash".to_string(),
            device_name: "Test Browser".to_string(),
            device_type: DeviceType::Browser,
        };

        let result = service.login(login_request).await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn test_login_user_not_found() {
        let service = create_service();

        let login_request = LoginRequest {
            email: "nonexistent@example.com".to_string(),
            master_password_hash: "test_hash".to_string(),
            device_name: "Test Browser".to_string(),
            device_type: DeviceType::Browser,
        };

        let result = service.login(login_request).await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn test_validate_session() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        let (user, session) = service.validate_session(response.session_id).await.unwrap();

        assert_eq!(user.email, "test@example.com");
        assert_eq!(session.id, response.session_id);
    }

    #[tokio::test]
    async fn test_validate_invalid_session() {
        let service = create_service();

        let result = service.validate_session(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_logout() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        service.logout(response.session_id).await.unwrap();

        let result = service.validate_session(response.session_id).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_get_devices() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        let devices = service.get_devices(response.session_id).await.unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Test Browser");
    }

    #[tokio::test]
    async fn test_get_devices_requires_valid_session() {
        let service = create_service();

        let result = service.get_devices(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_revoke_device() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response1 = service.register(request).await.unwrap();

        // Login from another device to have a second device
        let login_request = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            device_name: "Second Device".to_string(),
            device_type: DeviceType::Mobile,
        };
        let response2 = service.login(login_request).await.unwrap();

        // Get devices from first session
        let devices = service.get_devices(response1.session_id).await.unwrap();
        assert_eq!(devices.len(), 2);

        // Find the second device (not the one from response1's session)
        let second_device = devices.iter().find(|d| d.name == "Second Device").unwrap();

        // Revoke the second device from first session
        service.revoke_device(response1.session_id, second_device.id).await.unwrap();

        // First session should still be valid
        assert!(service.validate_session(response1.session_id).await.is_ok());

        // Second session should be invalidated (its device was revoked)
        let result = service.validate_session(response2.session_id).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_revoke_device_requires_valid_session() {
        let service = create_service();

        let result = service.revoke_device(SessionId::new(), DeviceId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_cannot_revoke_current_device() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        let devices = service.get_devices(response.session_id).await.unwrap();
        let current_device_id = devices[0].id;

        // Try to revoke current device - should fail
        let result = service.revoke_device(response.session_id, current_device_id).await;
        assert!(matches!(result, Err(AuthError::CannotRevokeCurrentDevice)));

        // Session should still be valid
        assert!(service.validate_session(response.session_id).await.is_ok());
    }

    #[tokio::test]
    async fn test_account_lockout() {
        let repo = Arc::new(InMemoryRepository::new());
        let config = AuthConfig {
            max_failed_attempts: 3,
            lockout_duration: Duration::minutes(15),
            ..Default::default()
        };
        let service = AuthService::with_config(repo, config);

        let request = register_request("test@example.com");
        service.register(request).await.unwrap();

        let bad_login = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "wrong_hash".to_string(),
            device_name: "Test Browser".to_string(),
            device_type: DeviceType::Browser,
        };

        // Fail 3 times to trigger lockout
        for _ in 0..3 {
            let _ = service.login(bad_login.clone()).await;
        }

        // Wrong password still returns InvalidCredentials (prevents user enumeration)
        let result = service.login(bad_login).await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));

        // Correct password on locked account returns AccountLocked
        let good_login = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            device_name: "Test Browser".to_string(),
            device_type: DeviceType::Browser,
        };
        let result = service.login(good_login).await;
        assert!(matches!(result, Err(AuthError::AccountLocked)));
    }

    #[tokio::test]
    async fn test_logout_all() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response1 = service.register(request).await.unwrap();

        // Login from another device
        let login_request = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            device_name: "Mobile App".to_string(),
            device_type: DeviceType::Mobile,
        };
        let response2 = service.login(login_request).await.unwrap();

        // Logout from all devices using the first session
        service.logout_all(response1.session_id).await.unwrap();

        // Both sessions should be invalid
        assert!(service.validate_session(response1.session_id).await.is_err());
        assert!(service.validate_session(response2.session_id).await.is_err());
    }

    #[tokio::test]
    async fn test_logout_all_requires_valid_session() {
        let service = create_service();

        // Try to logout_all with invalid session
        let result = service.logout_all(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_recovery_requires_verification_hash() {
        let service = create_service();

        // Register with recovery
        let mut request = register_request("test@example.com");
        request.encrypted_recovery_key = Some(test_encrypted_blob());
        request.recovery_verification_hash = Some("test_recovery_hash".to_string());
        service.register(request).await.unwrap();

        // Try recovery with wrong hash
        let recovery_request = RecoveryRequest {
            email: "test@example.com".to_string(),
            recovery_verification_hash: "wrong_hash".to_string(),
            new_master_password_hash: "new_password_hash".to_string(),
            new_kdf_params: test_kdf_params(),
            new_encrypted_symmetric_key: test_encrypted_blob(),
            new_recovery_verification_hash: None,
            new_encrypted_recovery_key: None,
        };

        let result = service.recover_account(recovery_request).await;
        assert!(matches!(result, Err(AuthError::InvalidRecoveryMnemonic)));
    }

    #[tokio::test]
    async fn test_recovery_with_correct_hash() {
        let service = create_service();

        // Register with recovery
        let mut request = register_request("test@example.com");
        request.encrypted_recovery_key = Some(test_encrypted_blob());
        request.recovery_verification_hash = Some("test_recovery_hash".to_string());
        let response = service.register(request).await.unwrap();

        // Verify original session works
        assert!(service.validate_session(response.session_id).await.is_ok());

        // Recovery with correct hash
        let recovery_request = RecoveryRequest {
            email: "test@example.com".to_string(),
            recovery_verification_hash: "test_recovery_hash".to_string(),
            new_master_password_hash: "new_password_hash".to_string(),
            new_kdf_params: test_kdf_params(),
            new_encrypted_symmetric_key: test_encrypted_blob(),
            new_recovery_verification_hash: None,
            new_encrypted_recovery_key: None,
        };

        let recovery_response = service.recover_account(recovery_request).await.unwrap();

        // New session should work
        assert!(service.validate_session(recovery_response.session_id).await.is_ok());

        // Old session should be invalidated
        assert!(service.validate_session(response.session_id).await.is_err());
    }

    #[tokio::test]
    async fn test_rotate_key_requires_valid_session() {
        let service = create_service();

        // Try to rotate key with invalid session
        let result = service
            .rotate_key(
                SessionId::new(),
                "old_hash".to_string(),
                "new_hash".to_string(),
                test_kdf_params(),
                test_encrypted_blob(),
            )
            .await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_rotate_key_requires_old_password() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        // Try to rotate key with wrong old password
        let result = service
            .rotate_key(
                response.session_id,
                "wrong_old_hash".to_string(),
                "new_hash".to_string(),
                test_kdf_params(),
                test_encrypted_blob(),
            )
            .await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));

        // Session should still be valid (we didn't invalidate it)
        assert!(service.validate_session(response.session_id).await.is_ok());
    }

    #[tokio::test]
    async fn test_rotate_key_success() {
        let service = create_service();
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        // Rotate key with correct old password
        let result = service
            .rotate_key(
                response.session_id,
                "test_hash_abc123".to_string(), // correct old password
                "new_hash".to_string(),
                test_kdf_params(),
                test_encrypted_blob(),
            )
            .await;
        assert!(result.is_ok());

        // Old session should be invalidated
        assert!(service.validate_session(response.session_id).await.is_err());

        // Can login with new password
        let login = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "new_hash".to_string(),
            device_name: "New Device".to_string(),
            device_type: DeviceType::Browser,
        };
        assert!(service.login(login).await.is_ok());
    }

    #[tokio::test]
    async fn test_recovery_rate_limiting() {
        let repo = Arc::new(InMemoryRepository::new());
        let config = AuthConfig {
            max_failed_attempts: 3,
            lockout_duration: Duration::minutes(15),
            ..Default::default()
        };
        let service = AuthService::with_config(repo, config);

        // Register with recovery
        let mut request = register_request("test@example.com");
        request.encrypted_recovery_key = Some(test_encrypted_blob());
        request.recovery_verification_hash = Some("correct_hash".to_string());
        service.register(request).await.unwrap();

        // Try recovery with wrong hash 3 times
        for _ in 0..3 {
            let recovery_request = RecoveryRequest {
                email: "test@example.com".to_string(),
                recovery_verification_hash: "wrong_hash".to_string(),
                new_master_password_hash: "new_password_hash".to_string(),
                new_kdf_params: test_kdf_params(),
                new_encrypted_symmetric_key: test_encrypted_blob(),
                new_recovery_verification_hash: None,
                new_encrypted_recovery_key: None,
            };
            let _ = service.recover_account(recovery_request).await;
        }

        // Account should now be locked - even correct hash should fail
        let recovery_request = RecoveryRequest {
            email: "test@example.com".to_string(),
            recovery_verification_hash: "correct_hash".to_string(),
            new_master_password_hash: "new_password_hash".to_string(),
            new_kdf_params: test_kdf_params(),
            new_encrypted_symmetric_key: test_encrypted_blob(),
            new_recovery_verification_hash: None,
            new_encrypted_recovery_key: None,
        };
        let result = service.recover_account(recovery_request).await;
        assert!(matches!(result, Err(AuthError::AccountLocked)));
    }

    #[tokio::test]
    async fn test_recovery_no_user_enumeration() {
        let service = create_service();

        // Try recovery for non-existent user
        let recovery_request = RecoveryRequest {
            email: "nonexistent@example.com".to_string(),
            recovery_verification_hash: "some_hash".to_string(),
            new_master_password_hash: "new_password_hash".to_string(),
            new_kdf_params: test_kdf_params(),
            new_encrypted_symmetric_key: test_encrypted_blob(),
            new_recovery_verification_hash: None,
            new_encrypted_recovery_key: None,
        };

        // Should return InvalidRecoveryMnemonic, NOT UserNotFound
        let result = service.recover_account(recovery_request).await;
        assert!(matches!(result, Err(AuthError::InvalidRecoveryMnemonic)));
    }

    #[tokio::test]
    async fn test_recovery_no_recovery_enabled_enumeration() {
        let service = create_service();

        // Register WITHOUT recovery
        let request = register_request("test@example.com");
        service.register(request).await.unwrap();

        // Try recovery - should return InvalidRecoveryMnemonic, NOT RecoveryNotAvailable
        let recovery_request = RecoveryRequest {
            email: "test@example.com".to_string(),
            recovery_verification_hash: "some_hash".to_string(),
            new_master_password_hash: "new_password_hash".to_string(),
            new_kdf_params: test_kdf_params(),
            new_encrypted_symmetric_key: test_encrypted_blob(),
            new_recovery_verification_hash: None,
            new_encrypted_recovery_key: None,
        };

        let result = service.recover_account(recovery_request).await;
        assert!(matches!(result, Err(AuthError::InvalidRecoveryMnemonic)));
    }

    #[tokio::test]
    async fn test_idle_timeout() {
        use crate::repository::SessionRepository;

        let repo = Arc::new(InMemoryRepository::new());
        let config = AuthConfig {
            // Set idle timeout to 1 second for testing
            idle_timeout: Some(Duration::seconds(1)),
            ..Default::default()
        };
        let service = AuthService::with_config(Arc::clone(&repo), config);

        // Register and get session
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        // Session should be valid initially
        assert!(service.validate_session(response.session_id).await.is_ok());

        // Manually set the session's last_activity_at to far in the past
        // to simulate an idle session without waiting
        let session = repo.get_session(response.session_id).await.unwrap().unwrap();
        let mut old_session = session;
        old_session.last_activity_at = Utc::now() - Duration::hours(1);
        repo.update_session(&old_session).await.unwrap();

        // Session should now be considered idle-timed-out
        let result = service.validate_session(response.session_id).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));

        // The session should have been deleted
        let session = repo.get_session(response.session_id).await.unwrap();
        assert!(session.is_none());
    }

    #[tokio::test]
    async fn test_max_devices_reached() {
        let repo = Arc::new(InMemoryRepository::new());
        let config = AuthConfig {
            max_devices_per_user: 2, // Low limit for testing
            ..Default::default()
        };
        let service = AuthService::with_config(repo, config);

        // Register creates first device
        let request = register_request("test@example.com");
        service.register(request).await.unwrap();

        // Login from second device
        let login1 = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            device_name: "Device 2".to_string(),
            device_type: DeviceType::Mobile,
        };
        service.login(login1).await.unwrap();

        // Try to login from third device - should fail with MaxDevicesReached
        let login2 = LoginRequest {
            email: "test@example.com".to_string(),
            master_password_hash: "test_hash_abc123".to_string(),
            device_name: "Device 3".to_string(),
            device_type: DeviceType::Desktop,
        };
        let result = service.login(login2).await;
        assert!(matches!(result, Err(AuthError::MaxDevicesReached(2))));
    }

    #[tokio::test]
    async fn test_email_validation() {
        let service = create_service();

        // Empty email
        let mut request = register_request("");
        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No @ symbol
        request = register_request("testexample.com");
        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No domain
        request = register_request("test@");
        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No local part
        request = register_request("@example.com");
        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No dot in domain
        request = register_request("test@localhost");
        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // Domain starts with dot
        request = register_request("test@.example.com");
        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // Valid email should work
        request = register_request("test@example.com");
        let result = service.register(request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cleanup_stale_sessions() {
        use crate::repository::SessionRepository;

        let repo = Arc::new(InMemoryRepository::new());
        let config = AuthConfig {
            idle_timeout: Some(Duration::seconds(1)),
            ..Default::default()
        };
        let service = AuthService::with_config(Arc::clone(&repo), config);

        // Register to create a session
        let request = register_request("test@example.com");
        let response = service.register(request).await.unwrap();

        // Session exists
        assert!(repo.get_session(response.session_id).await.unwrap().is_some());

        // Make session idle
        let session = repo.get_session(response.session_id).await.unwrap().unwrap();
        let mut old_session = session;
        old_session.last_activity_at = Utc::now() - Duration::hours(1);
        repo.update_session(&old_session).await.unwrap();

        // Cleanup should remove it
        let cleaned = service.cleanup_stale_sessions().await.unwrap();
        assert_eq!(cleaned, 1);

        // Session should be gone
        assert!(repo.get_session(response.session_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_invalid_recovery_setup() {
        let service = create_service();

        // Only encrypted_recovery_key without verification_hash
        let mut request = register_request("test@example.com");
        request.encrypted_recovery_key = Some(test_encrypted_blob());
        request.recovery_verification_hash = None;

        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidRecoverySetup)));

        // Only verification_hash without encrypted_recovery_key
        let mut request = register_request("test2@example.com");
        request.encrypted_recovery_key = None;
        request.recovery_verification_hash = Some("hash".to_string());

        let result = service.register(request).await;
        assert!(matches!(result, Err(AuthError::InvalidRecoverySetup)));
    }
}
