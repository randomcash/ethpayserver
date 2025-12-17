//! Repository trait for auth data persistence.
//!
//! The auth crate defines the interface; the main application provides
//! the database implementation.

use async_trait::async_trait;
use webauthn_rs::prelude::{PasskeyAuthentication, PasskeyRegistration};

use crate::error::Result;
use crate::models::{
    Device, DeviceId, PasskeyCredential, PasskeyId, Session, SessionId, User, UserId,
    WalletChallenge, WalletCredential, WalletCredentialId,
};

/// Repository for user data persistence.
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user.
    async fn create_user(&self, user: &User) -> Result<()>;

    /// Find a user by ID.
    async fn get_user(&self, id: UserId) -> Result<Option<User>>;

    /// Find a user by email.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>>;

    /// Find a user by wallet address.
    /// The address should be checksummed (EIP-55) for consistent lookups.
    async fn get_user_by_wallet_address(&self, address: &str) -> Result<Option<User>>;

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

    /// Delete all devices for a user (used during account recovery).
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

/// Repository for wallet credential persistence.
#[async_trait]
pub trait WalletRepository: Send + Sync {
    /// Store a new wallet credential.
    async fn create_wallet(&self, credential: &WalletCredential) -> Result<()>;

    /// Get a wallet by ID.
    async fn get_wallet(&self, id: WalletCredentialId) -> Result<Option<WalletCredential>>;

    /// Get a wallet by address (checksummed).
    async fn get_wallet_by_address(&self, address: &str) -> Result<Option<WalletCredential>>;

    /// Get all wallets for a user.
    async fn get_wallets_for_user(&self, user_id: UserId) -> Result<Vec<WalletCredential>>;

    /// Update a wallet (e.g., last_used_at, name).
    async fn update_wallet(&self, credential: &WalletCredential) -> Result<()>;

    /// Deactivate a wallet (soft delete).
    async fn deactivate_wallet(&self, id: WalletCredentialId) -> Result<()>;

    /// Delete a wallet permanently.
    async fn delete_wallet(&self, id: WalletCredentialId) -> Result<()>;

    /// Delete all wallets for a user.
    async fn delete_all_wallets_for_user(&self, user_id: UserId) -> Result<()>;

    /// Count active wallets for a user.
    async fn count_active_wallets(&self, user_id: UserId) -> Result<u32>;
}

/// Repository for WebAuthn challenge state persistence.
///
/// Challenges are ephemeral and should expire after a short time (e.g., 5 minutes).
/// The implementor should handle cleanup of expired challenges.
#[async_trait]
pub trait ChallengeRepository: Send + Sync {
    /// Store a passkey registration challenge state along with the email.
    /// The email is stored to verify consistency between start and complete.
    async fn store_registration_challenge(
        &self,
        user_id: UserId,
        email: &str,
        state: PasskeyRegistration,
    ) -> Result<()>;

    /// Retrieve and consume a passkey registration challenge.
    /// Returns None if expired or not found.
    /// Returns (PasskeyRegistration, email) to verify email consistency.
    async fn take_registration_challenge(&self, user_id: UserId) -> Result<Option<(PasskeyRegistration, String)>>;

    /// Store a passkey authentication challenge state.
    async fn store_authentication_challenge(
        &self,
        user_id: UserId,
        state: PasskeyAuthentication,
    ) -> Result<()>;

    /// Retrieve and consume a passkey authentication challenge.
    /// Returns None if expired or not found.
    async fn take_authentication_challenge(&self, user_id: UserId) -> Result<Option<PasskeyAuthentication>>;

    /// Store a wallet authentication challenge state.
    async fn store_wallet_challenge(&self, user_id: UserId, challenge: WalletChallenge) -> Result<()>;

    /// Retrieve and consume a wallet authentication challenge.
    /// Returns None if expired or not found.
    async fn take_wallet_challenge(&self, user_id: UserId) -> Result<Option<WalletChallenge>>;

    /// Cleanup expired challenges.
    async fn cleanup_expired_challenges(&self) -> Result<u64>;
}

/// Repository for store data persistence.
#[async_trait]
pub trait StoreRepository: Send + Sync {
    /// Create a new store.
    async fn create_store(&self, store: &crate::store::Store) -> Result<()>;

    /// Get a store by ID.
    async fn get_store(&self, id: crate::store::StoreId) -> Result<Option<crate::store::Store>>;

    /// Get all stores for a user (as owner or member).
    async fn get_stores_for_user(&self, user_id: UserId) -> Result<Vec<crate::store::Store>>;

    /// Get stores owned by a user.
    async fn get_stores_owned_by(&self, user_id: UserId) -> Result<Vec<crate::store::Store>>;

    /// Update a store.
    async fn update_store(&self, store: &crate::store::Store) -> Result<()>;

    /// Archive a store (soft delete).
    async fn archive_store(&self, id: crate::store::StoreId) -> Result<()>;

    /// Delete a store permanently.
    async fn delete_store(&self, id: crate::store::StoreId) -> Result<()>;
}

/// Repository for store role data persistence.
#[async_trait]
pub trait StoreRoleRepository: Send + Sync {
    /// Create a new store role.
    async fn create_store_role(&self, role: &crate::store::StoreRole) -> Result<()>;

    /// Get a store role by ID.
    async fn get_store_role(&self, id: crate::store::StoreRoleId) -> Result<Option<crate::store::StoreRole>>;

    /// Get all roles for a store (including global defaults).
    async fn get_roles_for_store(&self, store_id: crate::store::StoreId) -> Result<Vec<crate::store::StoreRole>>;

    /// Get global default roles (store_id is NULL).
    async fn get_default_roles(&self) -> Result<Vec<crate::store::StoreRole>>;

    /// Get a default role by name.
    async fn get_default_role_by_name(&self, name: &str) -> Result<Option<crate::store::StoreRole>>;

    /// Update a store role.
    async fn update_store_role(&self, role: &crate::store::StoreRole) -> Result<()>;

    /// Delete a store role.
    async fn delete_store_role(&self, id: crate::store::StoreRoleId) -> Result<()>;
}

/// Repository for user-store relationship data persistence.
#[async_trait]
pub trait UserStoreRepository: Send + Sync {
    /// Add a user to a store with a specific role.
    async fn add_user_to_store(&self, user_store: &crate::store::UserStore) -> Result<()>;

    /// Get a user's membership in a specific store.
    async fn get_user_store(
        &self,
        user_id: UserId,
        store_id: crate::store::StoreId,
    ) -> Result<Option<crate::store::UserStore>>;

    /// Get all store memberships for a user.
    async fn get_user_stores(&self, user_id: UserId) -> Result<Vec<crate::store::UserStore>>;

    /// Get all users in a store.
    async fn get_store_users(&self, store_id: crate::store::StoreId) -> Result<Vec<crate::store::UserStore>>;

    /// Update a user's role in a store.
    async fn update_user_store(&self, user_store: &crate::store::UserStore) -> Result<()>;

    /// Remove a user from a store.
    async fn remove_user_from_store(&self, user_id: UserId, store_id: crate::store::StoreId) -> Result<()>;

    /// Check if a user has a specific permission in a store.
    /// This resolves the role and checks its permissions.
    async fn user_has_store_permission(
        &self,
        user_id: UserId,
        store_id: crate::store::StoreId,
        permission: &str,
    ) -> Result<bool>;

    /// Get a user's full store info (store + role details).
    async fn get_user_store_info(
        &self,
        user_id: UserId,
        store_id: crate::store::StoreId,
    ) -> Result<Option<crate::store::UserStoreInfo>>;

    /// Get all store infos for a user.
    async fn get_user_store_infos(&self, user_id: UserId) -> Result<Vec<crate::store::UserStoreInfo>>;
}

/// Combined repository trait for convenience.
/// Includes all auth-related repositories.
#[async_trait]
pub trait AuthRepository:
    UserRepository
    + DeviceRepository
    + SessionRepository
    + PasskeyRepository
    + WalletRepository
    + ChallengeRepository
    + StoreRepository
    + StoreRoleRepository
    + UserStoreRepository
{
}

// Blanket implementation for any type implementing all traits
impl<T> AuthRepository for T where
    T: UserRepository
        + DeviceRepository
        + SessionRepository
        + PasskeyRepository
        + WalletRepository
        + ChallengeRepository
        + StoreRepository
        + StoreRoleRepository
        + UserStoreRepository
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
        users_by_wallet: RwLock<HashMap<String, UserId>>,
        devices: RwLock<HashMap<DeviceId, Device>>,
        sessions: RwLock<HashMap<SessionId, Session>>,
        passkeys: RwLock<HashMap<PasskeyId, PasskeyCredential>>,
        wallets: RwLock<HashMap<WalletCredentialId, WalletCredential>>,
        /// Registration challenges stored with identifier (email or wallet) for consistency verification.
        registration_challenges: RwLock<HashMap<UserId, (PasskeyRegistration, String)>>,
        authentication_challenges: RwLock<HashMap<UserId, PasskeyAuthentication>>,
        wallet_challenges: RwLock<HashMap<UserId, WalletChallenge>>,
        // Store-related data
        stores: RwLock<HashMap<crate::store::StoreId, crate::store::Store>>,
        store_roles: RwLock<HashMap<crate::store::StoreRoleId, crate::store::StoreRole>>,
        user_stores: RwLock<HashMap<(UserId, crate::store::StoreId), crate::store::UserStore>>,
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
            let mut by_wallet = self.users_by_wallet.write().unwrap_or_else(|e| e.into_inner());

            // Check for existing user by email
            if let Some(ref email) = user.email {
                if by_email.contains_key(email) {
                    return Err(AuthError::UserExists(email.clone()));
                }
            }

            // Check for existing user by wallet
            if let Some(ref wallet) = user.primary_wallet_address {
                if by_wallet.contains_key(wallet) {
                    return Err(AuthError::UserExists(wallet.clone()));
                }
            }

            users.insert(user.id, user.clone());
            if let Some(ref email) = user.email {
                by_email.insert(email.clone(), user.id);
            }
            if let Some(ref wallet) = user.primary_wallet_address {
                by_wallet.insert(wallet.clone(), user.id);
            }
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

        async fn get_user_by_wallet_address(&self, address: &str) -> Result<Option<User>> {
            let by_wallet = self.users_by_wallet.read().unwrap_or_else(|e| e.into_inner());
            let users = self.users.read().unwrap_or_else(|e| e.into_inner());

            if let Some(id) = by_wallet.get(address) {
                Ok(users.get(id).cloned())
            } else {
                Ok(None)
            }
        }

        async fn update_user(&self, user: &User) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            let mut by_wallet = self.users_by_wallet.write().unwrap_or_else(|e| e.into_inner());

            if users.contains_key(&user.id) {
                // Update wallet index if primary wallet changed
                if let Some(old_user) = users.get(&user.id) {
                    if old_user.primary_wallet_address != user.primary_wallet_address {
                        if let Some(ref old_wallet) = old_user.primary_wallet_address {
                            by_wallet.remove(old_wallet);
                        }
                        if let Some(ref new_wallet) = user.primary_wallet_address {
                            by_wallet.insert(new_wallet.clone(), user.id);
                        }
                    }
                }
                users.insert(user.id, user.clone());
                Ok(())
            } else {
                Err(AuthError::UserNotFound(user.id.to_string()))
            }
        }

        async fn delete_user(&self, id: UserId) -> Result<()> {
            let mut users = self.users.write().unwrap_or_else(|e| e.into_inner());
            let mut by_email = self.users_by_email.write().unwrap_or_else(|e| e.into_inner());
            let mut by_wallet = self.users_by_wallet.write().unwrap_or_else(|e| e.into_inner());

            if let Some(user) = users.remove(&id) {
                if let Some(ref email) = user.email {
                    by_email.remove(email);
                }
                if let Some(ref wallet) = user.primary_wallet_address {
                    by_wallet.remove(wallet);
                }
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
    impl WalletRepository for InMemoryRepository {
        async fn create_wallet(&self, credential: &WalletCredential) -> Result<()> {
            let mut wallets = self.wallets.write().unwrap_or_else(|e| e.into_inner());
            let mut by_wallet = self.users_by_wallet.write().unwrap_or_else(|e| e.into_inner());

            // Check if wallet address already exists
            if wallets.values().any(|w| w.address == credential.address && w.is_active) {
                return Err(AuthError::WalletAlreadyRegistered);
            }

            wallets.insert(credential.id, credential.clone());

            // If this is the primary wallet, update the user lookup
            if credential.is_primary {
                by_wallet.insert(credential.address.clone(), credential.user_id);
            }
            Ok(())
        }

        async fn get_wallet(&self, id: WalletCredentialId) -> Result<Option<WalletCredential>> {
            let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
            Ok(wallets.get(&id).cloned())
        }

        async fn get_wallet_by_address(&self, address: &str) -> Result<Option<WalletCredential>> {
            let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
            Ok(wallets.values().find(|w| w.address == address && w.is_active).cloned())
        }

        async fn get_wallets_for_user(&self, user_id: UserId) -> Result<Vec<WalletCredential>> {
            let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
            Ok(wallets
                .values()
                .filter(|w| w.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn update_wallet(&self, credential: &WalletCredential) -> Result<()> {
            let mut wallets = self.wallets.write().unwrap_or_else(|e| e.into_inner());
            if wallets.contains_key(&credential.id) {
                wallets.insert(credential.id, credential.clone());
                Ok(())
            } else {
                Err(AuthError::WalletNotFound(credential.id.to_string()))
            }
        }

        async fn deactivate_wallet(&self, id: WalletCredentialId) -> Result<()> {
            let mut wallets = self.wallets.write().unwrap_or_else(|e| e.into_inner());
            if let Some(wallet) = wallets.get_mut(&id) {
                wallet.is_active = false;
                Ok(())
            } else {
                Err(AuthError::WalletNotFound(id.to_string()))
            }
        }

        async fn delete_wallet(&self, id: WalletCredentialId) -> Result<()> {
            let mut wallets = self.wallets.write().unwrap_or_else(|e| e.into_inner());
            wallets.remove(&id);
            Ok(())
        }

        async fn delete_all_wallets_for_user(&self, user_id: UserId) -> Result<()> {
            let mut wallets = self.wallets.write().unwrap_or_else(|e| e.into_inner());
            wallets.retain(|_, w| w.user_id != user_id);
            Ok(())
        }

        async fn count_active_wallets(&self, user_id: UserId) -> Result<u32> {
            let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
            Ok(wallets
                .values()
                .filter(|w| w.user_id == user_id && w.is_active)
                .count() as u32)
        }
    }

    #[async_trait]
    impl ChallengeRepository for InMemoryRepository {
        async fn store_registration_challenge(
            &self,
            user_id: UserId,
            email: &str,
            state: PasskeyRegistration,
        ) -> Result<()> {
            let mut challenges = self
                .registration_challenges
                .write()
                .unwrap_or_else(|e| e.into_inner());
            challenges.insert(user_id, (state, email.to_string()));
            Ok(())
        }

        async fn take_registration_challenge(
            &self,
            user_id: UserId,
        ) -> Result<Option<(PasskeyRegistration, String)>> {
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

        async fn store_wallet_challenge(&self, user_id: UserId, challenge: WalletChallenge) -> Result<()> {
            let mut challenges = self
                .wallet_challenges
                .write()
                .unwrap_or_else(|e| e.into_inner());
            challenges.insert(user_id, challenge);
            Ok(())
        }

        async fn take_wallet_challenge(&self, user_id: UserId) -> Result<Option<WalletChallenge>> {
            let mut challenges = self
                .wallet_challenges
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

    #[async_trait]
    impl StoreRepository for InMemoryRepository {
        async fn create_store(&self, store: &crate::store::Store) -> Result<()> {
            let mut stores = self.stores.write().unwrap_or_else(|e| e.into_inner());
            stores.insert(store.id, store.clone());
            Ok(())
        }

        async fn get_store(&self, id: crate::store::StoreId) -> Result<Option<crate::store::Store>> {
            let stores = self.stores.read().unwrap_or_else(|e| e.into_inner());
            Ok(stores.get(&id).cloned())
        }

        async fn get_stores_for_user(&self, user_id: UserId) -> Result<Vec<crate::store::Store>> {
            let stores = self.stores.read().unwrap_or_else(|e| e.into_inner());
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());

            let store_ids: Vec<_> = user_stores
                .keys()
                .filter(|(uid, _)| *uid == user_id)
                .map(|(_, sid)| *sid)
                .collect();

            Ok(stores
                .values()
                .filter(|s| store_ids.contains(&s.id) || s.owner_id == user_id)
                .cloned()
                .collect())
        }

        async fn get_stores_owned_by(&self, user_id: UserId) -> Result<Vec<crate::store::Store>> {
            let stores = self.stores.read().unwrap_or_else(|e| e.into_inner());
            Ok(stores
                .values()
                .filter(|s| s.owner_id == user_id)
                .cloned()
                .collect())
        }

        async fn update_store(&self, store: &crate::store::Store) -> Result<()> {
            let mut stores = self.stores.write().unwrap_or_else(|e| e.into_inner());
            if stores.contains_key(&store.id) {
                stores.insert(store.id, store.clone());
                Ok(())
            } else {
                Err(AuthError::StoreNotFound(store.id.to_string()))
            }
        }

        async fn archive_store(&self, id: crate::store::StoreId) -> Result<()> {
            let mut stores = self.stores.write().unwrap_or_else(|e| e.into_inner());
            if let Some(store) = stores.get_mut(&id) {
                store.archived = true;
                Ok(())
            } else {
                Err(AuthError::StoreNotFound(id.to_string()))
            }
        }

        async fn delete_store(&self, id: crate::store::StoreId) -> Result<()> {
            let mut stores = self.stores.write().unwrap_or_else(|e| e.into_inner());
            stores.remove(&id);
            Ok(())
        }
    }

    #[async_trait]
    impl StoreRoleRepository for InMemoryRepository {
        async fn create_store_role(&self, role: &crate::store::StoreRole) -> Result<()> {
            let mut roles = self.store_roles.write().unwrap_or_else(|e| e.into_inner());
            roles.insert(role.id, role.clone());
            Ok(())
        }

        async fn get_store_role(&self, id: crate::store::StoreRoleId) -> Result<Option<crate::store::StoreRole>> {
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());
            Ok(roles.get(&id).cloned())
        }

        async fn get_roles_for_store(&self, store_id: crate::store::StoreId) -> Result<Vec<crate::store::StoreRole>> {
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());
            Ok(roles
                .values()
                .filter(|r| r.store_id == Some(store_id) || r.store_id.is_none())
                .cloned()
                .collect())
        }

        async fn get_default_roles(&self) -> Result<Vec<crate::store::StoreRole>> {
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());
            Ok(roles
                .values()
                .filter(|r| r.store_id.is_none())
                .cloned()
                .collect())
        }

        async fn get_default_role_by_name(&self, name: &str) -> Result<Option<crate::store::StoreRole>> {
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());
            Ok(roles
                .values()
                .find(|r| r.store_id.is_none() && r.role == name)
                .cloned())
        }

        async fn update_store_role(&self, role: &crate::store::StoreRole) -> Result<()> {
            let mut roles = self.store_roles.write().unwrap_or_else(|e| e.into_inner());
            if roles.contains_key(&role.id) {
                roles.insert(role.id, role.clone());
                Ok(())
            } else {
                Err(AuthError::StoreRoleNotFound(role.id.to_string()))
            }
        }

        async fn delete_store_role(&self, id: crate::store::StoreRoleId) -> Result<()> {
            let mut roles = self.store_roles.write().unwrap_or_else(|e| e.into_inner());
            roles.remove(&id);
            Ok(())
        }
    }

    #[async_trait]
    impl UserStoreRepository for InMemoryRepository {
        async fn add_user_to_store(&self, user_store: &crate::store::UserStore) -> Result<()> {
            let mut user_stores = self.user_stores.write().unwrap_or_else(|e| e.into_inner());
            user_stores.insert((user_store.user_id, user_store.store_id), user_store.clone());
            Ok(())
        }

        async fn get_user_store(
            &self,
            user_id: UserId,
            store_id: crate::store::StoreId,
        ) -> Result<Option<crate::store::UserStore>> {
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());
            Ok(user_stores.get(&(user_id, store_id)).cloned())
        }

        async fn get_user_stores(&self, user_id: UserId) -> Result<Vec<crate::store::UserStore>> {
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());
            Ok(user_stores
                .values()
                .filter(|us| us.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn get_store_users(&self, store_id: crate::store::StoreId) -> Result<Vec<crate::store::UserStore>> {
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());
            Ok(user_stores
                .values()
                .filter(|us| us.store_id == store_id)
                .cloned()
                .collect())
        }

        async fn update_user_store(&self, user_store: &crate::store::UserStore) -> Result<()> {
            let mut user_stores = self.user_stores.write().unwrap_or_else(|e| e.into_inner());
            let key = (user_store.user_id, user_store.store_id);
            if user_stores.contains_key(&key) {
                user_stores.insert(key, user_store.clone());
                Ok(())
            } else {
                Err(AuthError::UserNotInStore)
            }
        }

        async fn remove_user_from_store(&self, user_id: UserId, store_id: crate::store::StoreId) -> Result<()> {
            let mut user_stores = self.user_stores.write().unwrap_or_else(|e| e.into_inner());
            user_stores.remove(&(user_id, store_id));
            Ok(())
        }

        async fn user_has_store_permission(
            &self,
            user_id: UserId,
            store_id: crate::store::StoreId,
            permission: &str,
        ) -> Result<bool> {
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());

            if let Some(user_store) = user_stores.get(&(user_id, store_id)) {
                if let Some(role) = roles.get(&user_store.store_role_id) {
                    return Ok(role.has_permission(permission));
                }
            }
            Ok(false)
        }

        async fn get_user_store_info(
            &self,
            user_id: UserId,
            store_id: crate::store::StoreId,
        ) -> Result<Option<crate::store::UserStoreInfo>> {
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());
            let stores = self.stores.read().unwrap_or_else(|e| e.into_inner());
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());

            if let Some(user_store) = user_stores.get(&(user_id, store_id)) {
                if let Some(store) = stores.get(&store_id) {
                    if let Some(role) = roles.get(&user_store.store_role_id) {
                        return Ok(Some(crate::store::UserStoreInfo {
                            store: crate::store::StoreInfo::from(store),
                            role: crate::store::StoreRoleInfo::from(role),
                        }));
                    }
                }
            }
            Ok(None)
        }

        async fn get_user_store_infos(&self, user_id: UserId) -> Result<Vec<crate::store::UserStoreInfo>> {
            let user_stores = self.user_stores.read().unwrap_or_else(|e| e.into_inner());
            let stores = self.stores.read().unwrap_or_else(|e| e.into_inner());
            let roles = self.store_roles.read().unwrap_or_else(|e| e.into_inner());

            let mut result = Vec::new();
            for user_store in user_stores.values().filter(|us| us.user_id == user_id) {
                if let Some(store) = stores.get(&user_store.store_id) {
                    if let Some(role) = roles.get(&user_store.store_role_id) {
                        result.push(crate::store::UserStoreInfo {
                            store: crate::store::StoreInfo::from(store),
                            role: crate::store::StoreRoleInfo::from(role),
                        });
                    }
                }
            }
            Ok(result)
        }
    }
}
