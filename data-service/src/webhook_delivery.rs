//! Webhook delivery tracking: this server's own record of what it sent to a
//! merchant's endpoint, and whether it arrived.
//!
//! `webhook_deliveries` is this server's table over this server's Redis
//! queue. `payserver-commons` already declares a `WebhookDeliveryReader` /
//! `WebhookDeliveryWriter` pair, but its `CreateDeliveryParams` describes an
//! HTTP-response-log keyed by `store_id` (`http_status`, `response_body`,
//! `latency_ms`, `success`) — a different shape from the columns this table
//! actually has (`store_webhook_id`, `invoice_id`, `attempts`,
//! `max_attempts`, `last_error`), and nothing anywhere implements it. Rather
//! than force that shape onto this schema, or take on the three-step commons
//! dance to change it, this trait lives here next to the schema it actually
//! matches — the same reasoning as [`crate::PayoutClaimReader`].
//!
//! One row per queued job, not per HTTP attempt: [`WebhookDeliveryWriter::upsert_delivery`]
//! is keyed by the job's own id, so a job that fails twice before succeeding
//! still leaves a single row with `attempts = 3`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use types::RepositoryResult;

/// Delivery status, mirroring the `status` column and the partial index on
/// `status IN ('pending', 'retrying')`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookDeliveryStatus {
    /// Queued, not yet attempted.
    Pending,
    /// At least one attempt failed and another is scheduled.
    Retrying,
    /// A subscriber returned 2xx.
    Delivered,
    /// Every attempt was used and none succeeded.
    Failed,
}

impl WebhookDeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Retrying => "retrying",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for WebhookDeliveryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WebhookDeliveryStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "retrying" => Ok(Self::Retrying),
            "delivered" => Ok(Self::Delivered),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown webhook delivery status: {other}")),
        }
    }
}

/// One row of `webhook_deliveries`, joined with `store_webhooks` for the
/// store it belongs to — a delivery row itself carries only
/// `store_webhook_id`, and every caller of [`WebhookDeliveryReader`] needs to
/// check store ownership.
#[derive(Debug, Clone)]
pub struct WebhookDeliveryData {
    pub id: Uuid,
    pub store_webhook_id: Uuid,
    pub store_id: Uuid,
    pub invoice_id: String,
    pub event_type: String,
    pub status: WebhookDeliveryStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    /// Text produced by the merchant's endpoint or the HTTP client trying to
    /// reach it — attacker-influenced from the subscriber's point of view.
    /// Never render this as HTML.
    pub last_error: Option<String>,
    /// The exact payload that was queued, so a replay can resend the
    /// original event rather than a snapshot of the invoice's current state.
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What one write to `webhook_deliveries` needs.
pub struct UpsertDeliveryParams {
    /// The queued job's own id. Every write for the same job reuses it, so
    /// retries update one row instead of inserting a new one.
    pub id: Uuid,
    pub store_webhook_id: Uuid,
    pub invoice_id: String,
    pub event_type: String,
    pub status: WebhookDeliveryStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub payload: serde_json::Value,
}

/// Write operations for webhook deliveries.
#[async_trait]
pub trait WebhookDeliveryWriter: Send + Sync {
    /// Insert the row for a newly queued job, or update it in place for a
    /// later attempt of the same job (matched on `params.id`).
    async fn upsert_delivery(&self, params: UpsertDeliveryParams) -> RepositoryResult<()>;
}

/// Read operations for webhook deliveries, always scoped to a store.
#[async_trait]
pub trait WebhookDeliveryReader: Send + Sync {
    /// A single delivery, for replay or detail views. Callers must check
    /// `store_id` against the store they have already verified the caller
    /// owns — this does not take a store id to scope by, since the id alone
    /// does not say which store it belongs to.
    async fn get_delivery(&self, id: Uuid) -> RepositoryResult<Option<WebhookDeliveryData>>;

    /// Recent deliveries for one invoice, newest first.
    async fn list_deliveries_for_invoice(
        &self,
        invoice_id: &str,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDeliveryData>)>;

    /// Recent deliveries for a store, newest first. Scoped through
    /// `store_webhooks.store_id`, since a delivery row only carries
    /// `store_webhook_id`.
    async fn list_deliveries_for_store(
        &self,
        store_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDeliveryData>)>;
}
