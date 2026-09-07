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

/// One day of payment volume for a single asset.
///
/// Dates arrive as ISO `YYYY-MM-DD`; the client only ever displays them, so
/// there is no date library on this side to parse them with.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DailyVolume {
    pub date: String,
    /// Volume in whole units of the asset (e.g. `"0.75"` ETH).
    pub amount: String,
    pub payment_count: i64,
}

/// Payment volume for one asset over the analytics window.
///
/// `total_amount` and `daily[].amount` are in this asset's own units and are
/// not comparable across assets — see the server's `dashboard::analytics` for
/// why no combined total exists.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetVolume {
    pub asset_symbol: String,
    pub total_amount: String,
    pub payment_count: i64,
    /// Share of the window's payment *count*, not of value.
    pub share_percent: f64,
    pub daily: Vec<DailyVolume>,
}

/// Payment analytics for the dashboard volume chart and methods breakdown.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardAnalytics {
    pub days: u32,
    pub start_date: String,
    pub end_date: String,
    pub total_payments: i64,
    /// Busiest asset first. Empty for an account that has received nothing.
    pub assets: Vec<AssetVolume>,
}
