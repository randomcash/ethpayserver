//! HTTP client for ethpayserver API.

use gloo_net::http::{Request, RequestBuilder};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use super::{
    ApiKeyListResponse, CheckoutResponse, CreateApiKeyRequest, CreateApiKeyResponsePayload,
    CreateInvoiceRequest, CreatePaymentMethodRequest, CreateStoreRequest, DashboardStats, Invoice,
    InvoiceListResponse, InvoiceStatusResponse, Payment, PaymentListResponse, Store,
    StorePaymentMethod, StoreWebhook, UpdatePaymentMethodRequest, UpdateStoreRequest,
    UpdateWebhookRequest, UserInfo, Wallet,
};

/// API client errors.
#[derive(Error, Debug, Clone)]
pub enum ApiError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("HTTP error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Unauthorized")]
    Unauthorized,
}

/// API client for ethpayserver.
#[derive(Clone)]
pub struct EvmApiClient {
    base_url: String,
    token: Option<String>,
}

impl EvmApiClient {
    /// Create a new API client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
        }
    }

    /// Create a client for public endpoints (no auth header sent).
    ///
    /// Uses a same-origin relative base URL. Call `with_token` to authenticate later.
    pub fn unauthenticated() -> Self {
        Self::new("")
    }

    /// Set the authorization token.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// Build a request with authentication.
    fn build_request(&self, method: &str, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let builder = match method {
            "GET" => Request::get(&url),
            "POST" => Request::post(&url),
            "PUT" => Request::put(&url),
            "DELETE" => Request::delete(&url),
            "PATCH" => Request::patch(&url),
            _ => Request::get(&url),
        };

        let builder = if let Some(ref token) = self.token {
            builder.header("Authorization", &format!("Bearer {}", token))
        } else {
            builder
        };

        builder.header("Content-Type", "application/json")
    }

    /// Make a GET request.
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let request = self
            .build_request("GET", path)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a POST request.
    async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self
            .build_request("POST", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a PUT request.
    async fn put<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self
            .build_request("PUT", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a DELETE request.
    async fn delete(&self, path: &str) -> Result<(), ApiError> {
        let request = self
            .build_request("DELETE", path)
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

    /// Handle response and parse JSON.
    async fn handle_response<T: DeserializeOwned>(
        &self,
        response: gloo_net::http::Response,
    ) -> Result<T, ApiError> {
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

        response
            .json()
            .await
            .map_err(|e| ApiError::Parse(e.to_string()))
    }

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
    // Invoices
    // =========================================================================

    /// List invoices with filters and pagination.
    ///
    /// `store_id` is required for non-admin users.
    pub async fn list_invoices(
        &self,
        store_id: &str,
        status: Option<&str>,
        currency: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<InvoiceListResponse, ApiError> {
        let mut query = format!(
            "/api/invoices?store_id={}",
            js_sys::encode_uri_component(store_id)
        );
        if let Some(s) = status {
            query.push_str(&format!("&status={}", js_sys::encode_uri_component(s)));
        }
        if let Some(c) = currency {
            query.push_str(&format!("&currency={}", js_sys::encode_uri_component(c)));
        }
        if let Some(l) = limit {
            query.push_str(&format!("&limit={}", l));
        }
        if let Some(o) = offset {
            query.push_str(&format!("&offset={}", o));
        }
        self.get(&query).await
    }

    /// Get an invoice by ID.
    pub async fn get_invoice(&self, id: &str) -> Result<Invoice, ApiError> {
        self.get(&format!("/api/invoices/{}", id)).await
    }

    /// Get public checkout data for an invoice (no auth required).
    pub async fn get_checkout(&self, invoice_id: &str) -> Result<CheckoutResponse, ApiError> {
        self.get(&format!("/checkout/{}", invoice_id)).await
    }

    /// Create a new invoice.
    pub async fn create_invoice(
        &self,
        request: &CreateInvoiceRequest,
    ) -> Result<Invoice, ApiError> {
        self.post("/api/invoices", request).await
    }

    /// Get payments for an invoice.
    pub async fn get_invoice_payments(&self, invoice_id: &str) -> Result<Vec<Payment>, ApiError> {
        self.get(&format!("/api/invoices/{}/payments", invoice_id))
            .await
    }

    /// Get invoice status (includes payment options and payments).
    pub async fn get_invoice_status(
        &self,
        invoice_id: &str,
    ) -> Result<InvoiceStatusResponse, ApiError> {
        self.get(&format!("/api/invoices/{}/status", invoice_id))
            .await
    }

    // =========================================================================
    // Payments (store-scoped)
    // =========================================================================

    /// List payments with filters and pagination.
    ///
    /// `store_id` is required for non-admin users.
    pub async fn list_payments(
        &self,
        store_id: &str,
        status: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<PaymentListResponse, ApiError> {
        let mut query = format!(
            "/api/payments?store_id={}",
            js_sys::encode_uri_component(store_id)
        );
        if let Some(s) = status {
            query.push_str(&format!("&status={}", js_sys::encode_uri_component(s)));
        }
        if let Some(l) = limit {
            query.push_str(&format!("&limit={}", l));
        }
        if let Some(o) = offset {
            query.push_str(&format!("&offset={}", o));
        }
        self.get(&query).await
    }

    /// Get a single payment by ID.
    pub async fn get_payment(&self, id: &str) -> Result<Payment, ApiError> {
        self.get(&format!("/api/payments/{}", id)).await
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_client_new() {
        let client = EvmApiClient::new("http://localhost:5000");
        assert_eq!(client.base_url, "http://localhost:5000");
        assert_eq!(client.token, None);
    }

    #[test]
    fn test_api_client_with_token() {
        let client =
            EvmApiClient::new("http://localhost:5000").with_token(Some("test-token".to_string()));

        assert_eq!(client.token, Some("test-token".to_string()));
    }

    #[test]
    fn test_api_error_display() {
        let err = ApiError::Http {
            status: 404,
            message: "Not Found".to_string(),
        };
        assert_eq!(err.to_string(), "HTTP error 404: Not Found");

        let err = ApiError::Unauthorized;
        assert_eq!(err.to_string(), "Unauthorized");
    }
}
