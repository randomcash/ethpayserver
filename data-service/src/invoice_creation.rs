//! Creating an invoice as one unit of work.
//!
//! Invoice creation writes three things: the invoice, a payment option per
//! accepted asset, and a watched address per option so the monitor knows to look
//! for money. Those used to be three independent trips to the database, each
//! committing on its own.
//!
//! A failure partway through therefore left a committed invoice with some of its
//! payment options, or none. That invoice sits in `Pending` until it expires,
//! quoting a customer an address nobody is watching, or quoting no address at
//! all. It also counts towards the store's dashboard. Nothing reports it,
//! because from the server's point of view every individual write succeeded.
//!
//! The comment at the old failure site read "invoice creation aborted". It was
//! not: the invoice had been committed several statements earlier.
//!
//! # What is deliberately *not* in the transaction
//!
//! The derivation counter. `allocate_derivation` advances it before an address
//! can be derived, and that advance must stand even when the rest rolls back.
//! Burning an index costs nothing - the next invoice simply uses the one after.
//! *Returning* an index risks handing the same address to two invoices, which is
//! the collision that was live on testnet and took a migration to clean up.
//!
//! So the rule is: an index, once issued, is spent. Atomicity covers the rows
//! that describe an invoice, not the counter that numbered it.

use async_trait::async_trait;
use types::{InvoiceData, PaymentOptionData, RepositoryResult};

/// Write an invoice and everything that makes it payable, or write nothing.
#[async_trait]
pub trait InvoiceCreationWriter: Send + Sync {
    /// Insert `invoice`, every option in `options`, and a watched address for
    /// each, in a single transaction.
    ///
    /// Each option must already carry its derived `payment_address`, `chain_id`
    /// and `token_address`; this writes what it is given and derives nothing.
    /// The watched addresses take their expiry from `invoice.expires_at`, which
    /// is the value the caller already holds - the per-option path used to
    /// re-read it from the database and fall back to "24 hours from now" when
    /// the row was not found, a guess that could only ever be wrong.
    ///
    /// On any error nothing is written, including the invoice.
    async fn create_invoice_with_options(
        &self,
        invoice: &InvoiceData,
        options: &[PaymentOptionData],
    ) -> RepositoryResult<()>;
}
