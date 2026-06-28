//! Invoice management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.
//!
//! Split into the list page ([`list`]), the detail page ([`detail`]), and the
//! shared formatting/icon helpers ([`helpers`]).

mod detail;
mod helpers;
mod list;
mod widgets;

pub use detail::InvoiceDetailPage;
pub use list::InvoicesPage;
