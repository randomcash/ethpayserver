//! Transactional outbox for webhook notification obligations.
//!
//! A payment row and the fact that it owes a webhook notification used to be
//! two independent writes, with nothing durable recorded between them: a
//! crash between the two left a committed payment with no obligation ever
//! enqueued, and no reclaim mechanism can replay an obligation that was
//! never written. [`crate::PaymentTxIndexWriter::upsert_with_tx_index_and_obligation`]
//! writes the payment row and an obligation row in one database transaction,
//! so the two always commit together or not at all. A background drain
//! reads undispatched obligations, turns each into an actual queued
//! delivery, and marks it dispatched.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use types::RepositoryResult;

/// A recorded-but-not-yet-dispatched webhook notification obligation.
///
/// Unlike a `webhook_deliveries` row, this always names the payment it is
/// about - `payment_id` is a `NOT NULL` foreign key, not an afterthought.
#[derive(Debug, Clone)]
pub struct WebhookObligation {
    pub id: Uuid,
    pub payment_id: Uuid,
    pub invoice_id: String,
    /// [`api_types::webhook::WebhookEventType::as_str`] wire name. Stored as
    /// plain text rather than the enum itself, so this crate need not depend
    /// on `api-types` for a single column.
    pub event_type: String,
    pub created_at: DateTime<Utc>,
}

/// Read the outbox for the drain step.
#[async_trait]
pub trait WebhookOutboxReader: Send + Sync {
    /// Undispatched obligations, oldest first, capped at `limit` per call so
    /// one drain tick cannot be swamped by a backlog.
    async fn get_undispatched_obligations(
        &self,
        limit: i64,
    ) -> RepositoryResult<Vec<WebhookObligation>>;
}

/// Write to the outbox.
#[async_trait]
pub trait WebhookOutboxWriter: Send + Sync {
    /// Record that an obligation has been handed to the delivery queue (or
    /// determined not to need one - no webhook configured, or suppressed by
    /// notification preferences), so the drain does not read it again.
    async fn mark_obligation_dispatched(&self, id: Uuid) -> RepositoryResult<()>;
}
