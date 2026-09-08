//! Shapes the server does not define, and presentation helpers over shared ones.
//!
//! Everything here is genuinely client-side. Anything the API actually speaks
//! belongs in `api-types` so both ends compile against one definition.

use serde::{Deserialize, Serialize};
use types::InvoiceStatus;

use super::Store;

/// How an invoice status is shown.
///
/// Labels and CSS classes are a rendering decision, so they stay here rather
/// than in the contract - the server has no opinion on what colour "expired" is.
pub trait InvoiceStatusExt {
    fn label(&self) -> &'static str;
    fn css_class(&self) -> &'static str;
}

impl InvoiceStatusExt for InvoiceStatus {
    fn label(&self) -> &'static str {
        match self {
            InvoiceStatus::Pending => "Pending",
            InvoiceStatus::Processing => "Processing",
            InvoiceStatus::PartiallyPaid => "Partially Paid",
            InvoiceStatus::Paid => "Paid",
            InvoiceStatus::Expired => "Expired",
            InvoiceStatus::Cancelled => "Cancelled",
            InvoiceStatus::Refunded => "Refunded",
            InvoiceStatus::LatePaid => "Late Paid",
        }
    }

    fn css_class(&self) -> &'static str {
        match self {
            InvoiceStatus::Pending => "badge badge-warning",
            InvoiceStatus::Processing => "badge badge-info",
            InvoiceStatus::PartiallyPaid => "badge badge-warning",
            InvoiceStatus::Paid => "badge badge-success",
            InvoiceStatus::Expired => "badge badge-error",
            InvoiceStatus::Cancelled => "badge badge-neutral",
            InvoiceStatus::Refunded => "badge badge-neutral",
            InvoiceStatus::LatePaid => "badge badge-info",
        }
    }
}

/// A role a user holds on a store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreRole {
    pub id: String,
    pub store_id: Option<String>,
    pub role: String,
    pub permissions: Vec<String>,
}

/// A store together with the caller's role on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStoreInfo {
    pub store: Store,
    pub role: StoreRole,
}
