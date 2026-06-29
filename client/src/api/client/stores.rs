//! Store API methods: stores, payment methods, webhooks, settings, token
//! policy, and wallets.

use super::{ApiError, EvmApiClient};
use crate::api::{
    CreatePaymentMethodRequest, CreateStoreRequest, SetTokenPolicyRequest, Store,
    StorePaymentMethod, StoreSettings, StoreWebhook, TokenPolicy, UpdatePaymentMethodRequest,
    UpdateStoreRequest, UpdateStoreSettingsRequest, UpdateWebhookRequest, Wallet,
};

impl EvmApiClient {
    // =========================================================================
    // Stores
    // =========================================================================

    /// List stores.
    pub async fn list_stores(&self) -> Result<Vec<Store>, ApiError> {
        self.get("/api/stores").await
    }

    /// Get a store by ID.
    pub async fn get_store(&self, id: &str) -> Result<Store, ApiError> {
        self.get(&format!("/api/stores/{}", id)).await
    }

    /// Create a new store.
    pub async fn create_store(&self, request: &CreateStoreRequest) -> Result<Store, ApiError> {
        self.post("/api/stores", request).await
    }

    /// Update a store.
    pub async fn update_store(
        &self,
        id: &str,
        request: &UpdateStoreRequest,
    ) -> Result<Store, ApiError> {
        self.put(&format!("/api/stores/{}", id), request).await
    }

    /// Delete a store.
    pub async fn delete_store(&self, id: &str) -> Result<(), ApiError> {
        self.delete(&format!("/api/stores/{}", id)).await
    }

    // =========================================================================
    // Payment Methods
    // =========================================================================

    /// List payment methods for a store.
    pub async fn list_payment_methods(
        &self,
        store_id: &str,
    ) -> Result<Vec<StorePaymentMethod>, ApiError> {
        self.get(&format!("/api/stores/{}/payment-methods", store_id))
            .await
    }

    /// Create a payment method for a store.
    pub async fn create_payment_method(
        &self,
        store_id: &str,
        request: &CreatePaymentMethodRequest,
    ) -> Result<StorePaymentMethod, ApiError> {
        self.post(
            &format!("/api/stores/{}/payment-methods", store_id),
            request,
        )
        .await
    }

    /// Update a payment method.
    pub async fn update_payment_method(
        &self,
        store_id: &str,
        method_id: &str,
        request: &UpdatePaymentMethodRequest,
    ) -> Result<StorePaymentMethod, ApiError> {
        self.put(
            &format!("/api/stores/{}/payment-methods/{}", store_id, method_id),
            request,
        )
        .await
    }

    /// Delete a payment method.
    pub async fn delete_payment_method(
        &self,
        store_id: &str,
        method_id: &str,
    ) -> Result<(), ApiError> {
        self.delete(&format!(
            "/api/stores/{}/payment-methods/{}",
            store_id, method_id
        ))
        .await
    }

    // =========================================================================
    // Webhooks
    // =========================================================================

    /// Get webhook configuration for a store.
    pub async fn get_store_webhook(&self, store_id: &str) -> Result<StoreWebhook, ApiError> {
        self.get(&format!("/api/stores/{}/webhook", store_id)).await
    }

    /// Configure (create or update) webhook for a store.
    /// Returns the webhook with the secret visible (only time it's shown).
    pub async fn configure_store_webhook(
        &self,
        store_id: &str,
        request: &UpdateWebhookRequest,
    ) -> Result<StoreWebhook, ApiError> {
        self.put(&format!("/api/stores/{}/webhook", store_id), request)
            .await
    }

    /// Delete webhook configuration for a store.
    pub async fn delete_store_webhook(&self, store_id: &str) -> Result<(), ApiError> {
        self.delete(&format!("/api/stores/{}/webhook", store_id))
            .await
    }

    // =========================================================================
    // Store Settings
    // =========================================================================

    /// Get store settings.
    pub async fn get_store_settings(&self, store_id: &str) -> Result<StoreSettings, ApiError> {
        self.get(&format!("/api/stores/{}/settings", store_id))
            .await
    }

    /// Update store settings (partial update).
    pub async fn update_store_settings(
        &self,
        store_id: &str,
        request: &UpdateStoreSettingsRequest,
    ) -> Result<StoreSettings, ApiError> {
        self.patch(&format!("/api/stores/{}/settings", store_id), request)
            .await
    }

    // =========================================================================
    // Token Policy
    // =========================================================================

    /// Get the token policy for a store.
    pub async fn get_token_policy(&self, store_id: &str) -> Result<Option<TokenPolicy>, ApiError> {
        self.get(&format!("/api/stores/{}/token-policy", store_id))
            .await
    }

    /// Set (upsert) the token policy for a store.
    pub async fn set_token_policy(
        &self,
        store_id: &str,
        request: &SetTokenPolicyRequest,
    ) -> Result<TokenPolicy, ApiError> {
        self.put(&format!("/api/stores/{}/token-policy", store_id), request)
            .await
    }

    /// Delete the token policy for a store.
    pub async fn delete_token_policy(&self, store_id: &str) -> Result<(), ApiError> {
        self.delete(&format!("/api/stores/{}/token-policy", store_id))
            .await
    }

    // =========================================================================
    // Wallets
    // =========================================================================

    /// List wallets.
    pub async fn list_wallets(&self) -> Result<Vec<Wallet>, ApiError> {
        self.get("/api/wallets").await
    }

    /// Get a wallet by ID.
    pub async fn get_wallet(&self, id: &str) -> Result<Wallet, ApiError> {
        self.get(&format!("/api/wallets/{}", id)).await
    }
}
