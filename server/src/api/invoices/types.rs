use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

pub use api_types::{
    CreateInvoiceRequest, InvoiceListResponse, InvoiceResponse, InvoiceStatusResponse,
    PaymentListResponse, PaymentOptionResponse, PaymentResponse, TxHashLookupResponse,
};

/// Query parameters for listing payments.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListPaymentsQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status (confirmed, pending).
    pub status: Option<String>,
    /// Free-text search over tx hash, invoice id, asset symbol and sender.
    ///
    /// Applied in SQL so `total` counts the filtered set (RCS-231). Blank is
    /// no filter.
    pub search: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Query parameters for listing invoices.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListInvoicesQuery {
    /// Filter by store ID.
    pub store_id: Option<Uuid>,
    /// Filter by status.
    pub status: Option<String>,
    /// Filter by currency.
    pub currency: Option<String>,
    /// Free-text search over id, currency, amount and metadata.
    ///
    /// Applied in SQL so `total` counts the filtered set (RCS-231). Blank is
    /// no filter.
    pub search: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}
