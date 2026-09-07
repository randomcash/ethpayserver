//! Dashboard API endpoints.
//!
//! - [`stats`] — the invoice/payment counters behind the metric cards.
//! - [`analytics`] — the per-day, per-asset volume series behind the volume
//!   chart and the payment-methods breakdown (RCS-225).

mod analytics;
mod stats;

// Glob re-export so route registration and the `utoipa` path items generated
// beside each handler both resolve through `dashboard::`.
pub use analytics::*;
pub use stats::*;
