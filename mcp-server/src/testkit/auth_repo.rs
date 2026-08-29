//! Stub auth repository for the API key validation flow.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use auth::{ApiKey, ApiKeyId, ApiKeyRepository, AuthError, StoreRepository, UserId};
use types::{Store, StoreId};

use crate::api_key::hash_api_key;

/// The raw key whose SHA-256 hash [`test_api_key`] stores.
pub const RAW_KEY: &str = "ak_live_abc123";

/// Stub auth repository: holds one optional API key plus the stores its owner
/// can reach, and records `update_last_used` calls.
#[derive(Default)]
pub struct StubAuthRepo {
    api_key: Option<ApiKey>,
    stores: Vec<Store>,
    /// When set, `get_api_key_by_hash` fails instead of returning a key.
    lookup_fails: bool,
    last_used_calls: Mutex<Vec<ApiKeyId>>,
}

impl StubAuthRepo {
    pub fn with_key(api_key: ApiKey) -> Self {
        Self {
            api_key: Some(api_key),
            ..Default::default()
        }
    }

    pub fn with_store(mut self, store: Store) -> Self {
        self.stores.push(store);
        self
    }

    pub fn failing_lookup() -> Self {
        Self {
            lookup_fails: true,
            ..Default::default()
        }
    }

    pub fn last_used_calls(&self) -> Vec<ApiKeyId> {
        self.last_used_calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ApiKeyRepository for StubAuthRepo {
    async fn create_api_key(&self, _key: &ApiKey) -> auth::Result<()> {
        Ok(())
    }

    async fn get_api_key(&self, _id: ApiKeyId) -> auth::Result<Option<ApiKey>> {
        Ok(self.api_key.clone())
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> auth::Result<Option<ApiKey>> {
        if self.lookup_fails {
            return Err(AuthError::Repository("connection reset".to_string()));
        }
        Ok(self.api_key.clone().filter(|key| key.key_hash == key_hash))
    }

    async fn list_user_api_keys(&self, _user_id: UserId) -> auth::Result<Vec<ApiKey>> {
        Ok(self.api_key.clone().into_iter().collect())
    }

    async fn revoke_api_key(&self, _id: ApiKeyId) -> auth::Result<()> {
        Ok(())
    }

    async fn update_last_used(&self, id: ApiKeyId) -> auth::Result<()> {
        self.last_used_calls.lock().unwrap().push(id);
        Ok(())
    }

    async fn delete_api_key(&self, _id: ApiKeyId) -> auth::Result<()> {
        Ok(())
    }

    async fn delete_api_keys_for_user(&self, _user_id: UserId) -> auth::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl StoreRepository for StubAuthRepo {
    async fn create_store(&self, _store: &Store) -> auth::Result<()> {
        Ok(())
    }

    async fn get_store(&self, id: StoreId) -> auth::Result<Option<Store>> {
        Ok(self.stores.iter().find(|s| s.id == id).cloned())
    }

    async fn get_stores_for_user(&self, user_id: UserId) -> auth::Result<Vec<Store>> {
        Ok(self
            .stores
            .iter()
            .filter(|s| s.owner_id == user_id)
            .cloned()
            .collect())
    }

    async fn get_stores_owned_by(&self, user_id: UserId) -> auth::Result<Vec<Store>> {
        self.get_stores_for_user(user_id).await
    }

    async fn update_store(&self, _store: &Store) -> auth::Result<()> {
        Ok(())
    }

    async fn archive_store(&self, _id: StoreId) -> auth::Result<()> {
        Ok(())
    }

    async fn delete_store(&self, _id: StoreId) -> auth::Result<()> {
        Ok(())
    }
}

/// Build an active API key for `user_id` whose hash matches [`RAW_KEY`].
pub fn test_api_key(user_id: UserId) -> ApiKey {
    ApiKey {
        id: ApiKeyId(Uuid::new_v4()),
        user_id,
        name: "MCP agent key".to_string(),
        key_hash: hash_api_key(RAW_KEY),
        key_prefix: "ak_live_****c123".to_string(),
        is_active: true,
        created_at: Utc::now(),
        last_used_at: None,
        expires_at: None,
    }
}

pub fn test_store(owner_id: UserId) -> Store {
    Store {
        id: StoreId(Uuid::new_v4()),
        name: "Agent Store".to_string(),
        website: None,
        owner_id,
        archived: false,
        created_at: Utc::now(),
    }
}
