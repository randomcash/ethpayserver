//! Invoice repository traits.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::RepositoryResult;
use crate::store::StoreId;
use crate::traits::InvoiceData;
use crate::types::{InvoiceId, InvoiceStatus, Network};

/// Query parameters for listing invoices.
#[derive(Debug, Clone, Default)]
pub struct InvoiceQueryParams {
    pub store_id: Option<StoreId>,
    pub status: Option<InvoiceStatus>,
    pub network: Option<Network>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

impl InvoiceQueryParams {
    pub fn new() -> Self {
        Self {
            store_id: None,
            status: None,
            network: None,
            created_after: None,
            created_before: None,
            limit: 50,
            offset: 0,
        }
    }

    pub fn with_store_id(mut self, store_id: StoreId) -> Self {
        self.store_id = Some(store_id);
        self
    }

    pub fn with_status(mut self, status: InvoiceStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_offset(mut self, offset: i64) -> Self {
        self.offset = offset;
        self
    }
}

/// Read operations for invoices.
#[async_trait]
pub trait InvoiceReader: Send + Sync {
    /// Get an invoice by ID.
    async fn get(&self, id: &InvoiceId) -> RepositoryResult<Option<InvoiceData>>;

    /// Query invoices with pagination.
    /// Returns (total_count, invoices).
    async fn query(&self, params: &InvoiceQueryParams) -> RepositoryResult<(i64, Vec<InvoiceData>)>;

    /// Get all pending invoices that have expired.
    async fn get_expired(&self) -> RepositoryResult<Vec<InvoiceData>>;
}

/// Write operations for invoices.
#[async_trait]
pub trait InvoiceWriter: Send + Sync {
    /// Insert or update an invoice.
    async fn upsert(&self, invoice: &InvoiceData) -> RepositoryResult<()>;

    /// Update an invoice's status.
    async fn update_status(&self, id: &InvoiceId, status: InvoiceStatus) -> RepositoryResult<()>;

    /// Update the amount received for an invoice.
    async fn update_amount_received(&self, id: &InvoiceId, amount: &str) -> RepositoryResult<()>;
}

/// Combined invoice repository with full read/write access.
pub trait InvoiceRepository: InvoiceReader + InvoiceWriter {}

/// Blanket implementation for any type implementing both Reader and Writer.
impl<T: InvoiceReader + InvoiceWriter> InvoiceRepository for T {}
