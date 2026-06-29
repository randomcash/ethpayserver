//! Auth/user, dashboard, API key, and admin API methods.

use super::{ApiError, EvmApiClient};
use crate::api::{
    ApiKeyListResponse, CreateApiKeyRequest, CreateApiKeyResponsePayload, DashboardStats,
    RotateApiKeyResponse, ServerSettingsResponse, UpdateServerSettingsRequest,
    UpdateUserRoleRequest, UserInfo, UserListResponse,
};

impl EvmApiClient {
    // =========================================================================
    // Auth / User
    // =========================================================================

    /// Get the current authenticated user's info.
    pub async fn get_me(&self) -> Result<UserInfo, ApiError> {
        self.get("/api/auth/me").await
    }

    /// Log out the current session (server-side invalidation).
    pub async fn logout(&self) -> Result<(), ApiError> {
        let request = self
            .build_request("POST", "/api/auth/logout")
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }

        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }

        Ok(())
    }

    // =========================================================================
    // Dashboard
    // =========================================================================

    /// Get dashboard statistics.
    pub async fn get_dashboard_stats(&self) -> Result<DashboardStats, ApiError> {
        self.get("/api/dashboard/stats").await
    }

    // =========================================================================
    // API Keys
    // =========================================================================

    /// List API keys for the authenticated user.
    pub async fn list_api_keys(&self) -> Result<ApiKeyListResponse, ApiError> {
        self.get("/api/users/api-keys").await
    }

    /// Create a new API key.
    pub async fn create_api_key(
        &self,
        request: &CreateApiKeyRequest,
    ) -> Result<CreateApiKeyResponsePayload, ApiError> {
        self.post("/api/users/api-keys", request).await
    }

    /// Revoke an API key.
    pub async fn revoke_api_key(&self, id: &str) -> Result<(), ApiError> {
        self.delete(&format!("/api/users/api-keys/{}", id)).await
    }

    /// Rotate an API key (deprecates old, creates new).
    pub async fn rotate_api_key(&self, id: &str) -> Result<RotateApiKeyResponse, ApiError> {
        self.post_empty(&format!("/api/users/api-keys/{}/rotate", id))
            .await
    }

    // =========================================================================
    // Admin
    // =========================================================================

    /// List all users (admin only).
    pub async fn list_users(&self, offset: i64, limit: i64) -> Result<UserListResponse, ApiError> {
        self.get(&format!(
            "/api/admin/users?offset={}&limit={}",
            offset, limit
        ))
        .await
    }

    /// Update a user's role (admin only).
    pub async fn update_user_role(
        &self,
        user_id: &str,
        request: &UpdateUserRoleRequest,
    ) -> Result<(), ApiError> {
        self.patch_empty(&format!("/api/admin/users/{}/role", user_id), request)
            .await
    }

    /// Lock a user account (admin only).
    pub async fn lock_user(&self, user_id: &str) -> Result<(), ApiError> {
        self.post_empty_body(&format!("/api/admin/users/{}/lock", user_id))
            .await
    }

    /// Unlock a user account (admin only).
    pub async fn unlock_user(&self, user_id: &str) -> Result<(), ApiError> {
        self.post_empty_body(&format!("/api/admin/users/{}/unlock", user_id))
            .await
    }

    /// Get server settings (admin only).
    pub async fn get_server_settings(&self) -> Result<ServerSettingsResponse, ApiError> {
        self.get("/api/admin/settings").await
    }

    /// Update server settings (admin only).
    pub async fn update_server_settings(
        &self,
        request: &UpdateServerSettingsRequest,
    ) -> Result<(), ApiError> {
        self.put_empty("/api/admin/settings", request).await
    }
}
