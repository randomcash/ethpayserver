//! ApiKeyRepository implementation for PgDataService.

use async_trait::async_trait;

use auth::{ApiKey, ApiKeyId, ApiKeyRepository, UserId, error::Result};

use super::{PgDataService, api_key::PostgresApiKeyRepository};

#[async_trait]
impl ApiKeyRepository for PgDataService {
    async fn create_api_key(&self, key: &ApiKey) -> Result<()> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .create_api_key(key)
            .await
    }

    async fn get_api_key(&self, id: ApiKeyId) -> Result<Option<ApiKey>> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .get_api_key(id)
            .await
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .get_api_key_by_hash(key_hash)
            .await
    }

    async fn list_user_api_keys(&self, user_id: UserId) -> Result<Vec<ApiKey>> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .list_user_api_keys(user_id)
            .await
    }

    async fn revoke_api_key(&self, id: ApiKeyId) -> Result<()> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .revoke_api_key(id)
            .await
    }

    async fn update_last_used(&self, id: ApiKeyId) -> Result<()> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .update_last_used(id)
            .await
    }

    async fn delete_api_key(&self, id: ApiKeyId) -> Result<()> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .delete_api_key(id)
            .await
    }

    async fn delete_api_keys_for_user(&self, user_id: UserId) -> Result<()> {
        PostgresApiKeyRepository::new(self.pool.clone())
            .delete_api_keys_for_user(user_id)
            .await
    }
}
