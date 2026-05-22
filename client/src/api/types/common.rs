//! Common/shared API types.

use serde::{Deserialize, Serialize};

/// Paginated response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Dashboard statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardStats {
    pub total_invoices: i64,
    pub pending_invoices: i64,
    pub paid_invoices: i64,
    pub expired_invoices: i64,
    pub total_payments: i64,
    pub total_stores: u32,
}
