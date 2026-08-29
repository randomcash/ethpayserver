//! Payment API methods (store-scoped).

use super::{ApiError, EvmApiClient};
use crate::api::{Payment, PaymentListResponse};

impl EvmApiClient {
    /// List payments with filters and pagination.
    ///
    /// `store_id` of `None` means "All Stores": the server then returns
    /// payments across every store, which it only allows for server admins —
    /// any other caller gets `400 Bad Request` (RCS-171).
    pub async fn list_payments(
        &self,
        store_id: Option<&str>,
        status: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<PaymentListResponse, ApiError> {
        let mut params = Vec::new();
        if let Some(sid) = store_id {
            params.push(format!("store_id={}", js_sys::encode_uri_component(sid)));
        }
        if let Some(s) = status {
            params.push(format!("status={}", js_sys::encode_uri_component(s)));
        }
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        if let Some(o) = offset {
            params.push(format!("offset={}", o));
        }
        self.get(&format!("/api/payments?{}", params.join("&")))
            .await
    }

    /// Export payments as CSV text.
    ///
    /// `store_id` of `None` exports across all stores (admins only) — see
    /// [`Self::list_payments`].
    pub async fn export_payments_csv(
        &self,
        store_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<String, ApiError> {
        let mut params = Vec::new();
        if let Some(sid) = store_id {
            params.push(format!("store_id={}", js_sys::encode_uri_component(sid)));
        }
        if let Some(s) = status {
            params.push(format!("status={}", js_sys::encode_uri_component(s)));
        }
        self.get_text(&format!("/api/payments/export.csv?{}", params.join("&")))
            .await
    }

    /// Get a single payment by ID.
    pub async fn get_payment(&self, id: &str) -> Result<Payment, ApiError> {
        self.get(&format!("/api/payments/{}", id)).await
    }
}
