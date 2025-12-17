//! Repository traits for data persistence.
//!
//! This module defines the core repository traits that abstract database operations.
//! Each domain (invoices, payments, watched addresses) has:
//!
//! - **Reader trait** - Read-only operations (queries, lookups)
//! - **Writer trait** - Write operations (insert, update, delete)
//! - **Repository trait** - Combined read/write access (supertrait)
//!
//! # Example Usage
//!
//! ```ignore
//! // Read-only access for API queries
//! fn list_invoices(reader: &impl InvoiceReader) { ... }
//!
//! // Write access for processing
//! fn create_invoice(writer: &impl InvoiceWriter) { ... }
//!
//! // Full access
//! fn process_payment(repo: &impl InvoiceRepository) { ... }
//! ```
//!
//! # DataService
//!
//! For convenience, [`DataService`] combines all repository traits.

mod error;
mod invoice;
mod payment;
mod token;
mod watched_address;

pub use error::{RepositoryError, RepositoryResult};
pub use invoice::{InvoiceQueryParams, InvoiceReader, InvoiceRepository, InvoiceWriter};
pub use payment::{PaymentQueryParams, PaymentReader, PaymentRepository, PaymentWriter};
pub use token::{TokenData, TokenQueryParams, TokenReader, TokenRepository, TokenWriter};
pub use watched_address::{WatchedAddressReader, WatchedAddressRepository, WatchedAddressWriter};

/// Combined data service trait with full read/write access to all repositories.
///
/// This supertrait combines all repository traits for convenience when you need
/// access to all data operations. For more focused dependencies, use the
/// individual Reader/Writer traits.
pub trait DataService:
    InvoiceRepository + PaymentRepository + WatchedAddressRepository + TokenRepository
{
}

/// Blanket implementation: any type implementing all repository traits is a DataService.
impl<T> DataService for T where
    T: InvoiceRepository + PaymentRepository + WatchedAddressRepository + TokenRepository
{
}

/// Read-only data service trait.
///
/// Use this when you only need read access to all repositories.
pub trait DataServiceReader:
    InvoiceReader + PaymentReader + WatchedAddressReader + TokenReader
{
}

/// Blanket implementation for read-only access.
impl<T> DataServiceReader for T where
    T: InvoiceReader + PaymentReader + WatchedAddressReader + TokenReader
{
}

/// Write-only data service trait.
///
/// Use this when you only need write access to all repositories.
pub trait DataServiceWriter:
    InvoiceWriter + PaymentWriter + WatchedAddressWriter + TokenWriter
{
}

/// Blanket implementation for write-only access.
impl<T> DataServiceWriter for T where
    T: InvoiceWriter + PaymentWriter + WatchedAddressWriter + TokenWriter
{
}
