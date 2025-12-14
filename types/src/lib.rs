//! Common types and traits for the PayServer ecosystem.
//!
//! This crate provides the foundation for building payment servers that support
//! various blockchain networks. Each PayServer implementation (ethpayserver,
//! bitcoinpayserver, etc.) uses these common types and implements the core traits.
//!
//! # Architecture
//!
//! - `Network`: Enum of all supported blockchain networks
//! - `PayServer` trait: Core interface all payment servers implement
//! - `InvoiceData`, `PaymentData`: Generic data structures for invoices and payments
//! - Network-specific types (like ERC20 tokens) are defined in their respective PayServer crates

pub mod error;
pub mod traits;
pub mod types;

// Re-export commonly used types at the crate root for convenience.
pub use error::{PayServerError, PayServerResult};
pub use traits::{
    CreateInvoiceRequest, InvoiceData, InvoiceQuery, PayServer, PaymentData, PaymentEventPublisher,
    PaymentEventSubscriber, PaymentMonitor,
};
pub use types::{HealthStatus, InvoiceId, InvoiceStatus, Network, PaymentEvent};
