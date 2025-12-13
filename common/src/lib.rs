//! Common types and traits for the PayServer ecosystem.
//!
//! This crate provides the foundation for building payment servers that support
//! various cryptocurrencies and payment methods.

pub mod error;
pub mod traits;
pub mod types;

// Re-export commonly used types at the crate root for convenience.
pub use error::{PayServerError, PayServerResult};
pub use traits::{
    CreateInvoiceRequest, InvoiceQuery, PayServer, PaymentEventPublisher, PaymentEventSubscriber,
    PaymentMonitor,
};
pub use types::{
    Amount, Currency, HealthStatus, Invoice, InvoiceId, InvoiceStatus, Payment, PaymentEvent,
    PaymentMethod,
};
