//! Watched address repository traits.

use async_trait::async_trait;

use super::RepositoryResult;
use crate::types::{InvoiceId, Network};

/// Read operations for watched addresses.
#[async_trait]
pub trait WatchedAddressReader: Send + Sync {
    /// Get the invoice ID associated with an address.
    async fn get_invoice_id(
        &self,
        address: &str,
        network: Network,
    ) -> RepositoryResult<Option<InvoiceId>>;

    /// Get all active watched addresses.
    /// Returns tuples of (address, invoice_id, network).
    async fn get_active(&self) -> RepositoryResult<Vec<(String, InvoiceId, Network)>>;
}

/// Write operations for watched addresses.
#[async_trait]
pub trait WatchedAddressWriter: Send + Sync {
    /// Insert or update a watched address.
    async fn upsert(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
    ) -> RepositoryResult<()>;

    /// Remove a watched address.
    async fn remove(&self, address: &str, network: Network) -> RepositoryResult<()>;
}

/// Combined watched address repository with full read/write access.
pub trait WatchedAddressRepository: WatchedAddressReader + WatchedAddressWriter {}

/// Blanket implementation for any type implementing both Reader and Writer.
impl<T: WatchedAddressReader + WatchedAddressWriter> WatchedAddressRepository for T {}
