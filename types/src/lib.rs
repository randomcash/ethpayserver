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
//! - Repository traits: `InvoiceRepository`, `PaymentRepository`, `WatchedAddressRepository`
//! - `Store`, `StoreRole`, `UserStore`: Multi-tenant store management
//! - Network-specific types (like ERC20 tokens) are defined in their respective PayServer crates

pub mod error;
pub mod repositories;
pub mod store;
pub mod traits;
pub mod types;

// Re-export commonly used types at the crate root for convenience.
pub use error::{PayServerError, PayServerResult};
pub use repositories::{
    // Combined traits
    DataService, DataServiceReader, DataServiceWriter,
    // Invoice
    InvoiceQueryParams, InvoiceReader, InvoiceRepository, InvoiceWriter,
    // Payment
    PaymentQueryParams, PaymentReader, PaymentRepository, PaymentWriter,
    // Token
    TokenData, TokenQueryParams, TokenReader, TokenRepository, TokenWriter,
    // Watched Address
    WatchedAddressReader, WatchedAddressRepository, WatchedAddressWriter,
    // Errors
    RepositoryError, RepositoryResult,
};
pub use traits::{
    CreateInvoiceRequest, InvoiceData, InvoiceQuery, PayServer, PaymentData, PaymentEventPublisher,
    PaymentEventSubscriber, PaymentMonitor,
};
pub use types::{HealthStatus, InvoiceId, InvoiceStatus, Network, PaymentEvent, UserId};
pub use store::{Store, StoreId, StoreInfo};
