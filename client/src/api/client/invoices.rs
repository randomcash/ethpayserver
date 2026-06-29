//! Invoice API methods.

use super::{ApiError, EvmApiClient};
use crate::api::{
    CheckoutResponse, CreateInvoiceRequest, Invoice, InvoiceListResponse, InvoiceStatusResponse,
    Payment, TxHashLookupResponse,
};

impl EvmApiClient {
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
        self.get(&format!(
            "/api/checkout/{}",
            js_sys::encode_uri_component(invoice_id)
        ))
        .await
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

    /// Export invoices as CSV text.
    pub async fn export_invoices_csv(
        &self,
        store_id: &str,
        status: Option<&str>,
    ) -> Result<String, ApiError> {
        let mut query = format!("/api/invoices/export.csv?store_id={}", store_id);
        if let Some(s) = status {
            query.push_str(&format!("&status={}", s));
        }
        self.get_text(&query).await
    }

    /// Look up an invoice by transaction hash.
    pub async fn lookup_invoice_by_tx(
        &self,
        chain_id: u64,
        tx_hash: &str,
    ) -> Result<TxHashLookupResponse, ApiError> {
        self.get(&format!("/api/invoices/by-tx/{}/{}", chain_id, tx_hash))
            .await
    }
}
