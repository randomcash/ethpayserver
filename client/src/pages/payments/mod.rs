//! Payments pages - Stripe-inspired payment history view.
//!
//! Uses types from `crate::api::types` which mirror the backend.

mod detail;
mod format;
mod icons;
mod list;

pub use detail::PaymentDetailPage;
pub use list::PaymentsPage;
