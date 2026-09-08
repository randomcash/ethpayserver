//! Payment API methods (store-scoped).

use super::{ApiError, EvmApiClient};
use crate::api::{Payment, PaymentListResponse};

impl EvmApiClient {
    /// List payments with filters and pagination.
    ///
    /// `store_id` of `None` means "All Stores": the server then returns
    /// payments across every store, which it only allows for server admins —
    /// any other caller gets `400 Bad Request` (RCS-171).
    ///
    /// `search` is a free-text term over tx hash, invoice id, asset symbol and
    /// sender, applied in SQL so `total` counts the same rows the page shows
    /// (RCS-231).
    pub async fn list_payments(
        &self,
        store_id: Option<&str>,
        status: Option<&str>,
        search: Option<&str>,
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
        if let Some(q) = search {
            params.push(format!("search={}", js_sys::encode_uri_component(q)));
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
    ///
    /// `search` is passed for the same reason `status` is: the export shares
    /// the list's filter builders server-side, and an export that ignores the
    /// search box hands the merchant rows they cannot see (RCS-231).
    pub async fn export_payments_csv(
        &self,
        store_id: Option<&str>,
        status: Option<&str>,
        search: Option<&str>,
    ) -> Result<String, ApiError> {
        let mut params = Vec::new();
        if let Some(sid) = store_id {
            params.push(format!("store_id={}", js_sys::encode_uri_component(sid)));
        }
        if let Some(s) = status {
            params.push(format!("status={}", js_sys::encode_uri_component(s)));
        }
        if let Some(q) = search {
            params.push(format!("search={}", js_sys::encode_uri_component(q)));
        }
        self.get_text(&format!("/api/payments/export.csv?{}", params.join("&")))
            .await
    }

    /// Get a single payment by ID.
    pub async fn get_payment(&self, id: &str) -> Result<Payment, ApiError> {
        self.get(&format!("/api/payments/{}", id)).await
    }
}
