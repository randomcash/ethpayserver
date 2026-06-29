//! Payment API methods (store-scoped).

use super::{ApiError, EvmApiClient};
use crate::api::{Payment, PaymentListResponse};

impl EvmApiClient {
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

    /// Export payments as CSV text.
    pub async fn export_payments_csv(
        &self,
        store_id: &str,
        status: Option<&str>,
    ) -> Result<String, ApiError> {
        let mut query = format!("/api/payments/export.csv?store_id={}", store_id);
        if let Some(s) = status {
            query.push_str(&format!("&status={}", s));
        }
        self.get_text(&query).await
    }

    /// Get a single payment by ID.
    pub async fn get_payment(&self, id: &str) -> Result<Payment, ApiError> {
        self.get(&format!("/api/payments/{}", id)).await
    }
}
