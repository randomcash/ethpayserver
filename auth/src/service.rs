//! Authentication service with business logic.
//!
//! This service provides passkey-only authentication with BIP39 mnemonic recovery.
//! Password-based authentication has been removed for better security.

use std::sync::Arc;

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use url::Url;
use webauthn_rs::prelude::*;
use webauthn_rs::Webauthn;

use crate::error::{AuthError, Result};
use crate::models::{
    CompleteNewUserPasskeyRegistrationRequest, CompletePasskeyLoginRequest,
    CompletePasskeyRegistrationRequest, CompleteRecoveryRequest, Device, DeviceId, DeviceInfo,
    LoginResponse, PasskeyCredential, PasskeyId, PasskeyInfo, Session, SessionId,
    StartNewUserPasskeyRegistrationResponse, StartPasskeyLoginResponse,
    StartPasskeyRegistrationRequest, StartPasskeyRegistrationResponse, StartRecoveryRequest, User,
    UserId, UserInfo,
};
use crate::repository::{
    ChallengeRepository, DeviceRepository, PasskeyRepository, SessionRepository, UserRepository,
};

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

    /// Maximum passkeys per user.
    pub max_passkeys_per_user: u32,

    /// WebAuthn Relying Party ID (typically the domain, e.g., "example.com").
    pub rp_id: String,

    /// WebAuthn Relying Party name (displayed to user, e.g., "Example App").
    pub rp_name: String,

    /// WebAuthn Relying Party origin (the full URL, e.g., "https://example.com").
    pub rp_origin: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            max_failed_attempts: 5,
            lockout_duration: Duration::minutes(15),
            session_duration: Duration::hours(24),
            idle_timeout: Some(Duration::hours(2)), // 2 hour idle timeout
            max_devices_per_user: 10,
            max_passkeys_per_user: 10,
            rp_id: "localhost".to_string(),
            rp_name: "PayServer".to_string(),
            rp_origin: "http://localhost:8080".to_string(),
        }
    }
}

/// Authentication service handling registration, login, and session management.
pub struct AuthService<R>
where
    R: UserRepository + DeviceRepository + SessionRepository + PasskeyRepository + ChallengeRepository,
{
    repo: Arc<R>,
    config: AuthConfig,
    webauthn: Arc<Webauthn>,
}

/// Build a Webauthn instance from config.
fn build_webauthn(config: &AuthConfig) -> std::result::Result<Webauthn, WebauthnError> {
    let rp_origin = Url::parse(&config.rp_origin).map_err(|_| WebauthnError::Configuration)?;
    let builder = WebauthnBuilder::new(&config.rp_id, &rp_origin)?
        .rp_name(&config.rp_name);
    builder.build()
}

impl<R> AuthService<R>
where
    R: UserRepository + DeviceRepository + SessionRepository + PasskeyRepository + ChallengeRepository,
{
    /// Create a new auth service with the given repository and default config.
    ///
    /// # Panics
    /// Panics if the WebAuthn configuration is invalid.
    pub fn new(repo: Arc<R>) -> Self {
        let config = AuthConfig::default();
        let webauthn = build_webauthn(&config).expect("Invalid WebAuthn config");
        Self {
            repo,
            config,
            webauthn: Arc::new(webauthn),
        }
    }

    /// Create a new auth service with custom config.
    ///
    /// # Panics
    /// Panics if the WebAuthn configuration is invalid.
    pub fn with_config(repo: Arc<R>, config: AuthConfig) -> Self {
        let webauthn = build_webauthn(&config).expect("Invalid WebAuthn config");
        Self {
            repo,
            config,
            webauthn: Arc::new(webauthn),
        }
    }

    // Note: Password-based register() and login() methods have been removed.
    // Use passkey authentication instead:
    // - New users: start_new_user_passkey_registration() + complete_new_user_passkey_registration()
    // - Existing users: start_passkey_login() + complete_passkey_login()
    // - Account recovery: start_account_recovery() + complete_account_recovery()

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
        let (user_info, _session) = self.validate_session(session_id).await?;

        let devices = self.repo.get_devices_for_user(user_info.id).await?;

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
        let (user_info, session) = self.validate_session(session_id).await?;

        // Cannot revoke the device you're currently using
        if session.device_id == device_id {
            return Err(AuthError::CannotRevokeCurrentDevice);
        }

        // Verify the device belongs to this user
        let device = self
            .repo
            .get_device(device_id)
            .await?
            .ok_or(AuthError::DeviceNotFound(device_id.to_string()))?;

        if device.user_id != user_info.id {
            return Err(AuthError::DeviceNotFound(device_id.to_string()));
        }

        // Delete all sessions for this device
        self.repo.delete_sessions_for_device(device_id).await?;

        // Deactivate the device
        self.repo.deactivate_device(device_id).await
    }

    /// Start account recovery using BIP39 mnemonic.
    ///
    /// This is step 1 of the recovery process. After verifying the mnemonic,
    /// returns a passkey registration challenge so the user can register a new passkey.
    ///
    /// The client must:
    /// 1. Derive recovery key from mnemonic + email using Argon2id
    /// 2. Hash recovery key: base64(SHA-256(recovery_key)) for verification
    /// 3. Call this method with the hash
    /// 4. Use the returned challenge to register a new passkey
    /// 5. Call complete_account_recovery with the passkey credential
    ///
    /// # Rate Limiting
    /// Failed recovery attempts are tracked and the account is locked after
    /// too many failures.
    pub async fn start_account_recovery(
        &self,
        request: StartRecoveryRequest,
    ) -> Result<StartPasskeyRegistrationResponse> {
        validate_email(&request.email)?;
        let email_lower = request.email.to_lowercase();

        // Find user - return generic error to prevent user enumeration
        let user = match self.repo.get_user_by_email(&email_lower).await? {
            Some(u) => u,
            None => return Err(AuthError::InvalidRecoveryMnemonic),
        };

        // CRITICAL: Verify the client knows the recovery mnemonic by comparing hashes.
        // The client derives: base64(SHA-256(Argon2id(mnemonic, email)))
        // We compare this with the stored hash using constant-time comparison.
        let verification_matches = {
            let stored = Sha256::digest(user.recovery_verification_hash.as_bytes());
            let provided = Sha256::digest(request.recovery_verification_hash.as_bytes());
            stored.ct_eq(&provided).unwrap_u8() == 1
        };

        if !verification_matches {
            // Track failed recovery attempts
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
        if user.is_locked() {
            return Err(AuthError::AccountLocked);
        }

        // Verification passed - generate passkey registration challenge
        // We'll use the user's existing ID for the WebAuthn user handle

        // Generate WebAuthn registration challenge
        let (ccr, passkey_registration) = self
            .webauthn
            .start_passkey_registration(
                Uuid::from(user.id.0),
                &user.email,
                &user.email,
                None, // Don't exclude existing credentials - they'll be deleted on completion
            )
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Store the challenge state with email for consistency verification
        self.repo
            .store_registration_challenge(user.id, &email_lower, passkey_registration)
            .await?;

        Ok(StartPasskeyRegistrationResponse { options: ccr })
    }

    /// Complete account recovery.
    ///
    /// This is step 2 of the recovery process. After the user registers a new passkey,
    /// this method completes the recovery by:
    /// - Verifying the new passkey credential
    /// - Revoking all old passkeys
    /// - Revoking all existing sessions
    /// - Updating the user's encrypted symmetric key
    /// - Creating a new session
    ///
    /// # Atomicity
    /// This method performs multiple updates. The repository implementation should
    /// use a transaction to ensure atomicity.
    pub async fn complete_account_recovery(
        &self,
        email: &str,
        request: CompleteRecoveryRequest,
    ) -> Result<LoginResponse> {
        validate_email(email)?;
        let email_lower = email.to_lowercase();

        // Find user
        let user = self
            .repo
            .get_user_by_email(&email_lower)
            .await?
            .ok_or(AuthError::InvalidRecoveryMnemonic)?;

        // Retrieve the stored challenge state and verify email consistency
        let (passkey_registration, stored_email) = self
            .repo
            .take_registration_challenge(user.id)
            .await?
            .ok_or(AuthError::PasskeyChallengeExpired)?;

        // Verify the email matches what was used in start_account_recovery
        if stored_email != email_lower {
            return Err(AuthError::PasskeyChallengeExpired);
        }

        // Complete WebAuthn registration
        let passkey = self
            .webauthn
            .finish_passkey_registration(&request.credential, &passkey_registration)
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Recovery successful - reset failed attempts
        self.repo.reset_failed_logins(user.id).await?;

        // Revoke all existing passkeys
        self.repo.delete_all_passkeys_for_user(user.id).await?;

        // Revoke all existing devices
        self.repo.delete_all_devices_for_user(user.id).await?;

        // Revoke all existing sessions
        self.repo.delete_all_sessions_for_user(user.id).await?;

        // Update user with new encrypted key and recovery hash
        let mut updated_user = user.clone();
        updated_user.kdf_params = request.new_kdf_params.clone();
        updated_user.encrypted_symmetric_key = request.new_encrypted_symmetric_key.clone();
        updated_user.recovery_verification_hash = request.new_recovery_verification_hash.clone();
        updated_user.failed_login_attempts = 0;
        updated_user.locked_until = None;
        self.repo.update_user(&updated_user).await?;

        // Store the new passkey credential
        let passkey_cred = PasskeyCredential::new(user.id, request.passkey_name.clone(), passkey);
        self.repo.create_passkey(&passkey_cred).await?;

        // Create new device for recovery session
        let device = Device::new(
            user.id,
            request.device_name.clone(),
            request.device_type,
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
            device_id,
            encrypted_symmetric_key: request.new_encrypted_symmetric_key.clone(),
            kdf_params: request.new_kdf_params.clone(),
            email: email_lower,
            expires_at,
        })
    }

    // Note: rotate_key() has been removed. With passkey-only authentication,
    // there's no password to rotate. Key rotation would require mnemonic verification
    // and can be done through the recovery flow if needed.

    /// Clean up stale sessions (call periodically).
    ///
    /// Removes sessions that are either:
    /// - Absolutely expired (past expires_at)
    /// - Idle-timed-out (no activity for idle_timeout duration)
    pub async fn cleanup_stale_sessions(&self) -> Result<u64> {
        self.repo.delete_stale_sessions(self.config.idle_timeout).await
    }

    // ========================================================================
    // Passkey/WebAuthn Methods (RECOMMENDED authentication)
    // ========================================================================

    /// Start passkey registration for an existing user.
    ///
    /// Requires a valid session. Returns WebAuthn challenge options for the client
    /// to pass to the authenticator (e.g., browser's `navigator.credentials.create()`).
    pub async fn start_passkey_registration(
        &self,
        session_id: SessionId,
        request: StartPasskeyRegistrationRequest,
    ) -> Result<StartPasskeyRegistrationResponse> {
        // Validate session
        let (user_info, _session) = self.validate_session(session_id).await?;

        // Check passkey limit
        let passkey_count = self.repo.count_active_passkeys(user_info.id).await?;
        if passkey_count >= self.config.max_passkeys_per_user {
            return Err(AuthError::MaxPasskeysReached(self.config.max_passkeys_per_user));
        }

        // Get existing passkeys to exclude from registration
        let existing_passkeys = self.repo.get_passkeys_for_user(user_info.id).await?;
        let excluded_credentials: Vec<_> = existing_passkeys
            .iter()
            .filter(|p| p.is_active)
            .map(|p| p.passkey.cred_id().clone())
            .collect();

        // Generate WebAuthn registration challenge
        let (ccr, passkey_registration) = self
            .webauthn
            .start_passkey_registration(
                Uuid::from(user_info.id.0),
                &user_info.email,
                &user_info.email, // Display name same as email
                Some(excluded_credentials),
            )
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Store the challenge state with email for consistency verification
        self.repo
            .store_registration_challenge(user_info.id, &user_info.email, passkey_registration)
            .await?;

        // Note: passkey_name from request is intentionally unused here.
        // Client provides it again in CompletePasskeyRegistrationRequest.
        let _ = request.passkey_name;

        Ok(StartPasskeyRegistrationResponse { options: ccr })
    }

    /// Complete passkey registration.
    ///
    /// Validates the credential from the authenticator and stores the passkey.
    pub async fn complete_passkey_registration(
        &self,
        session_id: SessionId,
        request: CompletePasskeyRegistrationRequest,
    ) -> Result<PasskeyInfo> {
        // Validate session
        let (user_info, _session) = self.validate_session(session_id).await?;

        // Re-check passkey limit (race condition protection)
        let passkey_count = self.repo.count_active_passkeys(user_info.id).await?;
        if passkey_count >= self.config.max_passkeys_per_user {
            return Err(AuthError::MaxPasskeysReached(self.config.max_passkeys_per_user));
        }

        // Retrieve the stored challenge state and verify email consistency
        let (passkey_registration, stored_email) = self
            .repo
            .take_registration_challenge(user_info.id)
            .await?
            .ok_or(AuthError::PasskeyChallengeExpired)?;

        // Verify the email matches (should always match for existing user, but verify anyway)
        if stored_email != user_info.email {
            return Err(AuthError::PasskeyChallengeExpired);
        }

        // Complete WebAuthn registration
        let passkey = self
            .webauthn
            .finish_passkey_registration(&request.credential, &passkey_registration)
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Store the passkey credential
        let credential = PasskeyCredential::new(user_info.id, request.passkey_name, passkey);
        let passkey_info = PasskeyInfo::from(&credential);
        self.repo.create_passkey(&credential).await?;

        Ok(passkey_info)
    }

    /// Start passkey authentication.
    ///
    /// Returns WebAuthn challenge options for the client to pass to the authenticator.
    /// The user must provide their email to look up their registered passkeys.
    pub async fn start_passkey_login(&self, email: &str) -> Result<StartPasskeyLoginResponse> {
        validate_email(email)?;
        let email_lower = email.to_lowercase();

        // Find user - return generic error to prevent enumeration
        let user = self
            .repo
            .get_user_by_email(&email_lower)
            .await?
            .ok_or(AuthError::InvalidCredentials)?;

        // Check if account is locked - fail early before WebAuthn flow
        if user.is_locked() {
            return Err(AuthError::AccountLocked);
        }

        // Get user's passkeys
        let passkeys = self.repo.get_passkeys_for_user(user.id).await?;
        let active_passkeys: Vec<Passkey> = passkeys
            .iter()
            .filter(|p| p.is_active)
            .map(|p| p.passkey.clone())
            .collect();

        if active_passkeys.is_empty() {
            // User has no passkeys - return same error as invalid credentials
            return Err(AuthError::InvalidCredentials);
        }

        // Generate WebAuthn authentication challenge
        let (rcr, passkey_authentication) = self
            .webauthn
            .start_passkey_authentication(&active_passkeys)
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Store the challenge state
        self.repo
            .store_authentication_challenge(user.id, passkey_authentication)
            .await?;

        Ok(StartPasskeyLoginResponse { options: rcr })
    }

    /// Complete passkey authentication.
    ///
    /// Validates the credential from the authenticator and creates a session.
    pub async fn complete_passkey_login(
        &self,
        request: CompletePasskeyLoginRequest,
    ) -> Result<LoginResponse> {
        validate_email(&request.email)?;
        let email_lower = request.email.to_lowercase();

        // Find user
        let user = self
            .repo
            .get_user_by_email(&email_lower)
            .await?
            .ok_or(AuthError::InvalidCredentials)?;

        // Check if account is locked
        if user.is_locked() {
            return Err(AuthError::AccountLocked);
        }

        // Retrieve the stored challenge state
        let passkey_authentication = self
            .repo
            .take_authentication_challenge(user.id)
            .await?
            .ok_or(AuthError::PasskeyChallengeExpired)?;

        // Get user's passkeys for verification
        let passkeys = self.repo.get_passkeys_for_user(user.id).await?;
        let mut active_passkeys: Vec<_> = passkeys.into_iter().filter(|p| p.is_active).collect();

        // Complete WebAuthn authentication
        let auth_result = self
            .webauthn
            .finish_passkey_authentication(&request.credential, &passkey_authentication)
            .map_err(|_| {
                // WebAuthn verification failed - could be a technical issue or attack
                // Don't increment failed logins (passkey failures can't be brute-forced)
                AuthError::PasskeyVerificationFailed
            })?;

        // Update the passkey counter and last_used_at
        if let Some(cred) = active_passkeys
            .iter_mut()
            .find(|p| p.passkey.cred_id() == auth_result.cred_id())
        {
            cred.passkey.update_credential(&auth_result);
            cred.last_used_at = Some(Utc::now());
            self.repo.update_passkey(cred).await?;
        }

        // Reset failed login attempts on successful passkey auth
        self.repo.reset_failed_logins(user.id).await?;

        // Find existing device by ID, or create new device
        let device_id = if let Some(provided_device_id) = request.device_id {
            // Client provided a device ID - verify it belongs to this user and is active
            let device = self
                .repo
                .get_device(provided_device_id)
                .await?
                .ok_or(AuthError::DeviceNotFound(provided_device_id.to_string()))?;

            if device.user_id != user.id {
                // Device belongs to a different user - treat as not found
                return Err(AuthError::DeviceNotFound(provided_device_id.to_string()));
            }

            if !device.is_active {
                // Device was revoked - treat as not found, client should create new
                return Err(AuthError::DeviceNotFound(provided_device_id.to_string()));
            }

            // Update last_used_at
            let mut updated = device.clone();
            updated.last_used_at = Some(Utc::now());
            self.repo.update_device(&updated).await?;
            provided_device_id
        } else {
            // No device ID provided - create new device
            let active_count = self.repo.count_active_devices(user.id).await?;
            if active_count >= self.config.max_devices_per_user {
                return Err(AuthError::MaxDevicesReached(self.config.max_devices_per_user));
            }

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
            device_id,
            encrypted_symmetric_key: user.encrypted_symmetric_key,
            kdf_params: user.kdf_params,
            email: user.email,
            expires_at,
        })
    }

    /// Get all passkeys for the current user.
    ///
    /// Requires a valid session. Returns sanitized PasskeyInfo (without actual key material).
    pub async fn get_passkeys(&self, session_id: SessionId) -> Result<Vec<PasskeyInfo>> {
        let (user_info, _session) = self.validate_session(session_id).await?;

        let passkeys = self.repo.get_passkeys_for_user(user_info.id).await?;

        Ok(passkeys
            .iter()
            .filter(|p| p.is_active)
            .map(PasskeyInfo::from)
            .collect())
    }

    /// Revoke a passkey.
    ///
    /// Requires a valid session for authentication.
    pub async fn revoke_passkey(&self, session_id: SessionId, passkey_id: PasskeyId) -> Result<()> {
        let (user_info, _session) = self.validate_session(session_id).await?;

        // Verify the passkey belongs to this user
        let passkey = self
            .repo
            .get_passkey(passkey_id)
            .await?
            .ok_or(AuthError::PasskeyNotFound(passkey_id.to_string()))?;

        if passkey.user_id != user_info.id {
            return Err(AuthError::PasskeyNotFound(passkey_id.to_string()));
        }

        // Deactivate the passkey
        self.repo.deactivate_passkey(passkey_id).await
    }

    /// Register a new user with passkey (RECOMMENDED registration flow).
    ///
    /// This is a two-step process:
    /// 1. Client calls start_new_user_passkey_registration to get challenge + user_id
    /// 2. Client calls complete_new_user_passkey_registration with credential + user_id
    ///
    /// The client must generate the symmetric key and encrypt it appropriately.
    /// Since passkeys don't directly provide a decryption key, the client should:
    /// - Use the BIP39 mnemonic as the primary key source
    /// - Derive: recovery_key = Argon2id(mnemonic, email)
    /// - Use recovery_key to encrypt the symmetric key
    pub async fn start_new_user_passkey_registration(
        &self,
        email: &str,
    ) -> Result<StartNewUserPasskeyRegistrationResponse> {
        // Validate email
        validate_email(email)?;
        let email_lower = email.to_lowercase();

        // Check if user already exists
        if self.repo.get_user_by_email(&email_lower).await?.is_some() {
            return Err(AuthError::UserExists(email_lower));
        }

        // Generate a temporary user ID for the registration
        let user_id = UserId::new();

        // Generate WebAuthn registration challenge
        let (ccr, passkey_registration) = self
            .webauthn
            .start_passkey_registration(
                Uuid::from(user_id.0),
                &email_lower,
                &email_lower,
                None, // No excluded credentials for new user
            )
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Store the challenge state with email for consistency verification
        self.repo
            .store_registration_challenge(user_id, &email_lower, passkey_registration)
            .await?;

        Ok(StartNewUserPasskeyRegistrationResponse {
            options: ccr,
            user_id,
            email: email_lower,
        })
    }

    /// Complete new user registration with passkey.
    ///
    /// Creates the user account and passkey credential.
    /// The recovery_verification_hash is required for account recovery.
    ///
    /// # Atomicity
    /// This method creates user, passkey, device, and session in sequence.
    /// The repository implementation should use a transaction to ensure
    /// atomicity - if any step fails, all changes should be rolled back.
    pub async fn complete_new_user_passkey_registration(
        &self,
        request: CompleteNewUserPasskeyRegistrationRequest,
    ) -> Result<LoginResponse> {
        // Validate email
        validate_email(&request.email)?;
        let email_lower = request.email.to_lowercase();

        // Check if user already exists (race condition check)
        if self.repo.get_user_by_email(&email_lower).await?.is_some() {
            return Err(AuthError::UserExists(email_lower));
        }

        // Retrieve the stored challenge state and verify email matches
        let (passkey_registration, stored_email) = self
            .repo
            .take_registration_challenge(request.user_id)
            .await?
            .ok_or(AuthError::PasskeyChallengeExpired)?;

        // Verify the email matches what was used in start_new_user_passkey_registration
        // This prevents an attacker from starting with email_A and completing with email_B
        if stored_email != email_lower {
            return Err(AuthError::PasskeyChallengeExpired);
        }

        // Complete WebAuthn registration
        let passkey = self
            .webauthn
            .finish_passkey_registration(&request.credential, &passkey_registration)
            .map_err(|e| AuthError::WebAuthn(e.to_string()))?;

        // Create user with required recovery verification hash
        let mut user = User::new(
            email_lower.clone(),
            request.kdf_params.clone(),
            request.encrypted_symmetric_key.clone(),
            request.recovery_verification_hash.clone(),
        );
        // Use the user_id from the start request so it matches the passkey's user handle
        user.id = request.user_id;

        self.repo.create_user(&user).await?;

        // Store the passkey credential
        let passkey_cred = PasskeyCredential::new(user.id, request.passkey_name.clone(), passkey);
        self.repo.create_passkey(&passkey_cred).await?;

        // Create first device
        let device = Device::new(
            user.id,
            request.device_name.clone(),
            request.device_type,
            request.encrypted_symmetric_key.clone(),
            request.kdf_params.clone(),
        );
        let device_id = device.id;
        self.repo.create_device(&device).await?;

        // Create session
        let expires_at = Utc::now() + self.config.session_duration;
        let session = Session::with_expiration(user.id, device_id, expires_at);
        let session_id = session.id;
        self.repo.create_session(&session).await?;

        Ok(LoginResponse {
            session_id,
            device_id,
            encrypted_symmetric_key: request.encrypted_symmetric_key.clone(),
            kdf_params: request.kdf_params.clone(),
            email: email_lower,
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::inmemory::InMemoryRepository;

    fn create_service() -> AuthService<InMemoryRepository> {
        let repo = Arc::new(InMemoryRepository::new());
        AuthService::new(repo)
    }

    // ========================================================================
    // Session Validation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_validate_invalid_session() {
        let service = create_service();

        let result = service.validate_session(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_logout_requires_valid_session() {
        let service = create_service();

        // Logout with invalid session should succeed (idempotent)
        // but validate_session should still fail
        let _ = service.logout(SessionId::new()).await;
        let result = service.validate_session(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_logout_all_requires_valid_session() {
        let service = create_service();

        let result = service.logout_all(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    // ========================================================================
    // Device Management Tests
    // ========================================================================

    #[tokio::test]
    async fn test_get_devices_requires_valid_session() {
        let service = create_service();

        let result = service.get_devices(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_revoke_device_requires_valid_session() {
        let service = create_service();

        let result = service.revoke_device(SessionId::new(), DeviceId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    // ========================================================================
    // Passkey Tests
    // ========================================================================

    #[tokio::test]
    async fn test_get_passkeys_requires_valid_session() {
        let service = create_service();

        let result = service.get_passkeys(SessionId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_start_passkey_login_user_not_found() {
        let service = create_service();

        // Try passkey login for non-existent user
        let result = service.start_passkey_login("nonexistent@example.com").await;
        // Should return InvalidCredentials (not UserNotFound) to prevent enumeration
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn test_start_passkey_login_invalid_email() {
        let service = create_service();

        // Invalid email format should return InvalidEmail
        let result = service.start_passkey_login("invalid-email").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));
    }

    #[tokio::test]
    async fn test_revoke_passkey_requires_valid_session() {
        let service = create_service();

        let result = service.revoke_passkey(SessionId::new(), PasskeyId::new()).await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_start_passkey_registration_requires_valid_session() {
        let service = create_service();

        let request = StartPasskeyRegistrationRequest {
            passkey_name: "Test Passkey".to_string(),
        };

        let result = service
            .start_passkey_registration(SessionId::new(), request)
            .await;
        assert!(matches!(result, Err(AuthError::SessionInvalid)));
    }

    #[tokio::test]
    async fn test_start_new_user_passkey_registration_invalid_email() {
        let service = create_service();

        let result = service
            .start_new_user_passkey_registration("invalid-email")
            .await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));
    }

    #[tokio::test]
    async fn test_start_new_user_passkey_registration_valid_email() {
        let service = create_service();

        // Valid email should return a challenge and user_id
        let result = service
            .start_new_user_passkey_registration("test@example.com")
            .await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert!(!response.options.public_key.challenge.is_empty());
        // User ID should be returned so client can pass it back during completion
        assert!(!response.user_id.0.is_nil());
    }

    #[tokio::test]
    async fn test_start_new_user_passkey_registration_email_case_insensitive() {
        let service = create_service();

        // Start registration with uppercase email
        let result1 = service
            .start_new_user_passkey_registration("Test@Example.COM")
            .await;
        assert!(result1.is_ok());

        // Note: Completing registration would create the user, but we can't test
        // that without real WebAuthn credentials. This just tests the challenge
        // generation accepts various email formats.
    }

    // ========================================================================
    // Recovery Tests
    // ========================================================================

    #[tokio::test]
    async fn test_start_account_recovery_user_not_found() {
        let service = create_service();

        // Try recovery for non-existent user
        let request = StartRecoveryRequest {
            email: "nonexistent@example.com".to_string(),
            recovery_verification_hash: "some_hash".to_string(),
        };

        // Should return InvalidRecoveryMnemonic (not UserNotFound) to prevent enumeration
        let result = service.start_account_recovery(request).await;
        assert!(matches!(result, Err(AuthError::InvalidRecoveryMnemonic)));
    }

    #[tokio::test]
    async fn test_start_account_recovery_invalid_email() {
        let service = create_service();

        let request = StartRecoveryRequest {
            email: "invalid-email".to_string(),
            recovery_verification_hash: "some_hash".to_string(),
        };

        let result = service.start_account_recovery(request).await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));
    }

    // ========================================================================
    // Email Validation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_email_validation_via_passkey_registration() {
        let service = create_service();

        // Empty email
        let result = service.start_new_user_passkey_registration("").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No @ symbol
        let result = service.start_new_user_passkey_registration("testexample.com").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No domain
        let result = service.start_new_user_passkey_registration("test@").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No local part
        let result = service.start_new_user_passkey_registration("@example.com").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // No dot in domain
        let result = service.start_new_user_passkey_registration("test@localhost").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // Domain starts with dot
        let result = service.start_new_user_passkey_registration("test@.example.com").await;
        assert!(matches!(result, Err(AuthError::InvalidEmail(_))));

        // Valid email should work
        let result = service.start_new_user_passkey_registration("test@example.com").await;
        assert!(result.is_ok());
    }

    // Note: Full flow tests (registration, login, recovery, device management)
    // require real WebAuthn credentials which cannot be mocked easily.
    // These should be tested via integration tests with a WebAuthn testing library
    // or end-to-end tests with a real browser.
}
