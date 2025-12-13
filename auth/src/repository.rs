//! Repository trait for auth data persistence.
//!
//! The auth crate defines the interface; the main application provides
//! the database implementation.

use async_trait::async_trait;
use webauthn_rs::prelude::{PasskeyAuthentication, PasskeyRegistration};

use crate::error::Result;
use crate::models::{Device, DeviceId, PasskeyCredential, PasskeyId, Session, SessionId, User, UserId};

/// Repository for user data persistence.
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user.
    async fn create_user(&self, user: &User) -> Result<()>;

    /// Find a user by ID.
    async fn get_user(&self, id: UserId) -> Result<Option<User>>;

    /// Find a user by email.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>>;

    /// Update an existing user.
    async fn update_user(&self, user: &User) -> Result<()>;

    /// Delete a user and all associated data.
    async fn delete_user(&self, id: UserId) -> Result<()>;

    /// Increment failed login attempts.
    async fn increment_failed_logins(&self, id: UserId) -> Result<u32>;

    /// Reset failed login attempts (on successful login).
    async fn reset_failed_logins(&self, id: UserId) -> Result<()>;

    /// Lock user account until specified time.
    async fn lock_user(&self, id: UserId, until: chrono::DateTime<chrono::Utc>) -> Result<()>;

    /// Unlock user account.
    async fn unlock_user(&self, id: UserId) -> Result<()>;
}

/// Repository for device data persistence.
#[async_trait]
pub trait DeviceRepository: Send + Sync {
    /// Register a new device.
    async fn create_device(&self, device: &Device) -> Result<()>;

    /// Get a device by ID.
    async fn get_device(&self, id: DeviceId) -> Result<Option<Device>>;

    /// Get all devices for a user.
    async fn get_devices_for_user(&self, user_id: UserId) -> Result<Vec<Device>>;

    /// Update device (e.g., last_used_at).
    async fn update_device(&self, device: &Device) -> Result<()>;

    /// Deactivate a device (soft delete - keeps audit trail).
    async fn deactivate_device(&self, id: DeviceId) -> Result<()>;

    /// Delete a device permanently.
    async fn delete_device(&self, id: DeviceId) -> Result<()>;

    /// Delete all devices for a user (used during password reset/recovery).
    async fn delete_all_devices_for_user(&self, user_id: UserId) -> Result<()>;

    /// Count active devices for a user.
    async fn count_active_devices(&self, user_id: UserId) -> Result<u32>;
}

/// Repository for session data persistence.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Create a new session.
    async fn create_session(&self, session: &Session) -> Result<()>;

    /// Get a session by ID.
    async fn get_session(&self, id: SessionId) -> Result<Option<Session>>;

    /// Update session (e.g., last_activity_at).
    async fn update_session(&self, session: &Session) -> Result<()>;

    /// Delete a session (logout).
    async fn delete_session(&self, id: SessionId) -> Result<()>;

    /// Delete all sessions for a user (logout everywhere).
    async fn delete_all_sessions_for_user(&self, user_id: UserId) -> Result<()>;

    /// Delete all sessions for a device.
    async fn delete_sessions_for_device(&self, device_id: DeviceId) -> Result<()>;

    /// Delete expired sessions (cleanup job).
    /// Only checks absolute expiration. Use delete_stale_sessions for idle timeout.
    async fn delete_expired_sessions(&self) -> Result<u64>;

    /// Delete sessions that are expired OR idle-timed-out.
    /// idle_timeout: Duration after last_activity_at when session is considered stale.
    async fn delete_stale_sessions(
        &self,
        idle_timeout: Option<chrono::Duration>,
    ) -> Result<u64>;

    /// Get all active sessions for a user.
    async fn get_sessions_for_user(&self, user_id: UserId) -> Result<Vec<Session>>;
}

/// Repository for passkey credential persistence.
#[async_trait]
pub trait PasskeyRepository: Send + Sync {
    /// Store a new passkey credential.
    async fn create_passkey(&self, credential: &PasskeyCredential) -> Result<()>;

    /// Get a passkey by ID.
    async fn get_passkey(&self, id: PasskeyId) -> Result<Option<PasskeyCredential>>;

    /// Get all passkeys for a user.
    async fn get_passkeys_for_user(&self, user_id: UserId) -> Result<Vec<PasskeyCredential>>;

    /// Update a passkey (e.g., counter, last_used_at).
    async fn update_passkey(&self, credential: &PasskeyCredential) -> Result<()>;

    /// Deactivate a passkey (soft delete).
    async fn deactivate_passkey(&self, id: PasskeyId) -> Result<()>;

    /// Delete a passkey permanently.
    async fn delete_passkey(&self, id: PasskeyId) -> Result<()>;

    /// Delete all passkeys for a user.
    async fn delete_all_passkeys_for_user(&self, user_id: UserId) -> Result<()>;

    /// Count active passkeys for a user.
    async fn count_active_passkeys(&self, user_id: UserId) -> Result<u32>;
}

/// Repository for WebAuthn challenge state persistence.
///
/// Challenges are ephemeral and should expire after a short time (e.g., 5 minutes).
/// The implementor should handle cleanup of expired challenges.
#[async_trait]
pub trait ChallengeRepository: Send + Sync {
    /// Store a passkey registration challenge state.
    /// The key should be unique per user+session, e.g., user_id + timestamp.
    async fn store_registration_challenge(
        &self,
        user_id: UserId,
        state: PasskeyRegistration,
    ) -> Result<()>;

    /// Retrieve and consume a passkey registration challenge.
    /// Returns None if expired or not found.
    async fn take_registration_challenge(&self, user_id: UserId) -> Result<Option<PasskeyRegistration>>;

    /// Store a passkey authentication challenge state.
    async fn store_authentication_challenge(
        &self,
        user_id: UserId,
        state: PasskeyAuthentication,
    ) -> Result<()>;

    /// Retrieve and consume a passkey authentication challenge.
    /// Returns None if expired or not found.
    async fn take_authentication_challenge(&self, user_id: UserId) -> Result<Option<PasskeyAuthentication>>;

    /// Cleanup expired challenges.
    async fn cleanup_expired_challenges(&self) -> Result<u64>;
}

/// Combined repository trait for convenience.
/// Includes all auth-related repositories.
#[async_trait]
pub trait AuthRepository:
    UserRepository + DeviceRepository + SessionRepository + PasskeyRepository + ChallengeRepository
{
}

// Blanket implementation for any type implementing all traits
impl<T> AuthRepository for T where
    T: UserRepository + DeviceRepository + SessionRepository + PasskeyRepository + ChallengeRepository
{
}

#[cfg(test)]
pub mod inmemory {
    //! In-memory repository implementation for testing.
    //!
    //! Note: This implementation handles poisoned locks gracefully by recovering
    //! the data. In production, a proper database should be used instead.

    use std::collections::HashMap;
    use std::sync::RwLock;

    use super::*;
    use crate::error::AuthError;

    /// In-memory repository for testing.
    #[derive(Default)]
    pub struct InMemoryRepository {
        users: RwLock<HashMap<UserId, User>>,
        users_by_email: RwLock<HashMap<String, UserId>>,
        devices: RwLock<HashMap<DeviceId, Device>>,
        sessions: RwLock<HashMap<SessionId, Session>>,
        passkeys: RwLock<HashMap<PasskeyId, PasskeyCredential>>,
        registration_challenges: RwLock<HashMap<UserId, PasskeyRegistration>>,
        authentication_challenges: RwLock<HashMap<UserId, PasskeyAuthentication>>,
    }

    impl InMemoryRepository {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl UserRepository for InMemoryRepository {
        async fn create_user(&self, user: &User) -> Result<()> {
            // Handle poisoned locks gracefully by recovering the data
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            let mut by_email = self.users_by_email.write().unwrap_or_else(|e| e.into_inner());

            if by_email.contains_key(&user.email) {
                return Err(AuthError::UserExists(user.email.clone()));
            }

            users.insert(user.id, user.clone());
            by_email.insert(user.email.clone(), user.id);
            Ok(())
        }

        async fn get_user(&self, id: UserId) -> Result<Option<User>> {
            let users = self.users.read().unwrap_or_else(|e| e.into_inner());
            Ok(users.get(&id).cloned())
        }

        async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
            let by_email = self.users_by_email.read().unwrap_or_else(|e| e.into_inner());
            let users = self.users.read().unwrap_or_else(|e| e.into_inner());

            if let Some(id) = by_email.get(email) {
                Ok(users.get(id).cloned())
            } else {
                Ok(None)
            }
        }

        async fn update_user(&self, user: &User) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            if users.contains_key(&user.id) {
                users.insert(user.id, user.clone());
                Ok(())
            } else {
                Err(AuthError::UserNotFound(user.id.to_string()))
            }
        }

        async fn delete_user(&self, id: UserId) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            let mut by_email = self.users_by_email.write().unwrap_or_else(|e| e.into_inner());

            if let Some(user) = users.remove(&id) {
                by_email.remove(&user.email);
            }
            Ok(())
        }

        async fn increment_failed_logins(&self, id: UserId) -> Result<u32> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            if let Some(user) = users.get_mut(&id) {
                user.failed_login_attempts += 1;
                Ok(user.failed_login_attempts)
            } else {
                Err(AuthError::UserNotFound(id.to_string()))
            }
        }

        async fn reset_failed_logins(&self, id: UserId) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            if let Some(user) = users.get_mut(&id) {
                user.failed_login_attempts = 0;
                Ok(())
            } else {
                Err(AuthError::UserNotFound(id.to_string()))
            }
        }

        async fn lock_user(&self, id: UserId, until: chrono::DateTime<chrono::Utc>) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            if let Some(user) = users.get_mut(&id) {
                user.locked_until = Some(until);
                Ok(())
            } else {
                Err(AuthError::UserNotFound(id.to_string()))
            }
        }

        async fn unlock_user(&self, id: UserId) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            if let Some(user) = users.get_mut(&id) {
                user.locked_until = None;
                Ok(())
            } else {
                Err(AuthError::UserNotFound(id.to_string()))
            }
        }
    }

    #[async_trait]
    impl DeviceRepository for InMemoryRepository {
        async fn create_device(&self, device: &Device) -> Result<()> {
            let mut devices = self.devices.write().unwrap_or_else(|e| e.into_inner());
            devices.insert(device.id, device.clone());
            Ok(())
        }

        async fn get_device(&self, id: DeviceId) -> Result<Option<Device>> {
            let devices = self.devices.read().unwrap_or_else(|e| e.into_inner());
            Ok(devices.get(&id).cloned())
        }

        async fn get_devices_for_user(&self, user_id: UserId) -> Result<Vec<Device>> {
            let devices = self.devices.read().unwrap_or_else(|e| e.into_inner());
            Ok(devices
                .values()
                .filter(|d| d.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn update_device(&self, device: &Device) -> Result<()> {
            let mut devices = self.devices.write().unwrap_or_else(|e| e.into_inner());
            if devices.contains_key(&device.id) {
                devices.insert(device.id, device.clone());
                Ok(())
            } else {
                Err(AuthError::DeviceNotFound(device.id.to_string()))
            }
        }

        async fn deactivate_device(&self, id: DeviceId) -> Result<()> {
            let mut devices = self.devices.write().unwrap_or_else(|e| e.into_inner());
            if let Some(device) = devices.get_mut(&id) {
                device.is_active = false;
                Ok(())
            } else {
                Err(AuthError::DeviceNotFound(id.to_string()))
            }
        }

        async fn delete_device(&self, id: DeviceId) -> Result<()> {
            let mut devices = self.devices.write().unwrap_or_else(|e| e.into_inner());
            devices.remove(&id);
            Ok(())
        }

        async fn delete_all_devices_for_user(&self, user_id: UserId) -> Result<()> {
            let mut devices = self.devices.write().unwrap_or_else(|e| e.into_inner());
            devices.retain(|_, d| d.user_id != user_id);
            Ok(())
        }

        async fn count_active_devices(&self, user_id: UserId) -> Result<u32> {
            let devices = self.devices.read().unwrap_or_else(|e| e.into_inner());
            Ok(devices
                .values()
                .filter(|d| d.user_id == user_id && d.is_active)
                .count() as u32)
        }
    }

    #[async_trait]
    impl SessionRepository for InMemoryRepository {
        async fn create_session(&self, session: &Session) -> Result<()> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            sessions.insert(session.id, session.clone());
            Ok(())
        }

        async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
            let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
            Ok(sessions.get(&id).cloned())
        }

        async fn update_session(&self, session: &Session) -> Result<()> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            if sessions.contains_key(&session.id) {
                sessions.insert(session.id, session.clone());
                Ok(())
            } else {
                Err(AuthError::SessionInvalid)
            }
        }

        async fn delete_session(&self, id: SessionId) -> Result<()> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            sessions.remove(&id);
            Ok(())
        }

        async fn delete_all_sessions_for_user(&self, user_id: UserId) -> Result<()> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            sessions.retain(|_, s| s.user_id != user_id);
            Ok(())
        }

        async fn delete_sessions_for_device(&self, device_id: DeviceId) -> Result<()> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            sessions.retain(|_, s| s.device_id != device_id);
            Ok(())
        }

        async fn delete_expired_sessions(&self) -> Result<u64> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            let now = chrono::Utc::now();
            let before = sessions.len();
            sessions.retain(|_, s| s.expires_at > now);
            Ok((before - sessions.len()) as u64)
        }

        async fn delete_stale_sessions(
            &self,
            idle_timeout: Option<chrono::Duration>,
        ) -> Result<u64> {
            let mut sessions = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            let now = chrono::Utc::now();
            let before = sessions.len();

            sessions.retain(|_, s| {
                // Keep if not expired
                if s.expires_at <= now {
                    return false;
                }

                // Keep if no idle timeout OR not idle-timed-out
                if let Some(idle) = idle_timeout {
                    let idle_deadline = s.last_activity_at + idle;
                    if now > idle_deadline {
                        return false;
                    }
                }

                true
            });

            Ok((before - sessions.len()) as u64)
        }

        async fn get_sessions_for_user(&self, user_id: UserId) -> Result<Vec<Session>> {
            let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
            Ok(sessions
                .values()
                .filter(|s| s.user_id == user_id)
                .cloned()
                .collect())
        }
    }

    #[async_trait]
    impl PasskeyRepository for InMemoryRepository {
        async fn create_passkey(&self, credential: &PasskeyCredential) -> Result<()> {
            let mut passkeys = self.passkeys.write().unwrap_or_else(|e| e.into_inner());
            passkeys.insert(credential.id, credential.clone());
            Ok(())
        }

        async fn get_passkey(&self, id: PasskeyId) -> Result<Option<PasskeyCredential>> {
            let passkeys = self.passkeys.read().unwrap_or_else(|e| e.into_inner());
            Ok(passkeys.get(&id).cloned())
        }

        async fn get_passkeys_for_user(&self, user_id: UserId) -> Result<Vec<PasskeyCredential>> {
            let passkeys = self.passkeys.read().unwrap_or_else(|e| e.into_inner());
            Ok(passkeys
                .values()
                .filter(|p| p.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn update_passkey(&self, credential: &PasskeyCredential) -> Result<()> {
            let mut passkeys = self.passkeys.write().unwrap_or_else(|e| e.into_inner());
            if passkeys.contains_key(&credential.id) {
                passkeys.insert(credential.id, credential.clone());
                Ok(())
            } else {
                Err(AuthError::PasskeyNotFound(credential.id.to_string()))
            }
        }

        async fn deactivate_passkey(&self, id: PasskeyId) -> Result<()> {
            let mut passkeys = self.passkeys.write().unwrap_or_else(|e| e.into_inner());
            if let Some(passkey) = passkeys.get_mut(&id) {
                passkey.is_active = false;
                Ok(())
            } else {
                Err(AuthError::PasskeyNotFound(id.to_string()))
            }
        }

        async fn delete_passkey(&self, id: PasskeyId) -> Result<()> {
            let mut passkeys = self.passkeys.write().unwrap_or_else(|e| e.into_inner());
            passkeys.remove(&id);
            Ok(())
        }

        async fn delete_all_passkeys_for_user(&self, user_id: UserId) -> Result<()> {
            let mut passkeys = self.passkeys.write().unwrap_or_else(|e| e.into_inner());
            passkeys.retain(|_, p| p.user_id != user_id);
            Ok(())
        }

        async fn count_active_passkeys(&self, user_id: UserId) -> Result<u32> {
            let passkeys = self.passkeys.read().unwrap_or_else(|e| e.into_inner());
            Ok(passkeys
                .values()
                .filter(|p| p.user_id == user_id && p.is_active)
                .count() as u32)
        }
    }

    #[async_trait]
    impl ChallengeRepository for InMemoryRepository {
        async fn store_registration_challenge(
            &self,
            user_id: UserId,
            state: PasskeyRegistration,
        ) -> Result<()> {
            let mut challenges = self
                .registration_challenges
                .write()
                .unwrap_or_else(|e| e.into_inner());
            challenges.insert(user_id, state);
            Ok(())
        }

        async fn take_registration_challenge(
            &self,
            user_id: UserId,
        ) -> Result<Option<PasskeyRegistration>> {
            let mut challenges = self
                .registration_challenges
                .write()
                .unwrap_or_else(|e| e.into_inner());
            Ok(challenges.remove(&user_id))
        }

        async fn store_authentication_challenge(
            &self,
            user_id: UserId,
            state: PasskeyAuthentication,
        ) -> Result<()> {
            let mut challenges = self
                .authentication_challenges
                .write()
                .unwrap_or_else(|e| e.into_inner());
            challenges.insert(user_id, state);
            Ok(())
        }

        async fn take_authentication_challenge(
            &self,
            user_id: UserId,
        ) -> Result<Option<PasskeyAuthentication>> {
            let mut challenges = self
                .authentication_challenges
                .write()
                .unwrap_or_else(|e| e.into_inner());
            Ok(challenges.remove(&user_id))
        }

        async fn cleanup_expired_challenges(&self) -> Result<u64> {
            // In-memory implementation doesn't track expiration times
            // Real implementations should track timestamps and clean up old challenges
            Ok(0)
        }
    }
}
