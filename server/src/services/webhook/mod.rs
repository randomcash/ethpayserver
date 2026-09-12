//! Webhook notification service.
//!
//! Delivers webhook notifications to merchants when invoice status changes.
//! Uses Redis as a job queue with exponential backoff retries.
//!
//! # The event vocabulary is a contract
//!
//! [`WebhookEventType`] is the complete list of events a subscriber can ever
//! receive, and every variant of it is emitted by some code path in this
//! repository. Adding a variant means wiring an emission in the same change;
//! a variant that nothing can produce is a promise to subscribers that is
//! silently never kept.
//!
//! # Delivery is at-least-once
//!
//! A queued job is retried on any non-2xx response or transport error, on a
//! Stripe-like backoff (1m, 5m, 30m, 2h, 12h, 24h — see [`WebhookJob`]), and
//! the queue is a Redis sorted set from which a worker reads and then removes
//! the job. A subscriber that returns 2xx after a network failure, or a
//! handler that re-runs after this server restarts mid-transition, will
//! therefore see the same logical event more than once. There is no
//! at-most-once mode and no ordering guarantee between events for different
//! invoices.
//!
//! Subscribers must be idempotent. Every payload carries
//! [`WebhookPayload::idempotency_key`], derived from the event's identity
//! rather than from a random id or a send-time clock, so the same logical
//! event always carries the same key: record the keys you have processed and
//! drop repeats. It is also sent as the `X-Webhook-Idempotency-Key` header, so
//! a subscriber can dedupe before parsing the body.
//!
//! `event_id` is *not* that key — it identifies one queued delivery and a
//! re-emission of the same logical event gets a fresh one.
//!
//! # Payload compatibility
//!
//! Payloads carry an explicit [`WEBHOOK_PAYLOAD_VERSION`] in the `version`
//! field. The rule is **additive-only**:
//!
//! - New fields may be added at any time without bumping the version.
//!   Subscribers must ignore fields they do not recognise.
//! - New [`WebhookEventType`] variants may be added without bumping the
//!   version. Subscribers must ignore event types they do not recognise
//!   rather than failing the delivery.
//! - Removing a field, renaming one, changing its type, or changing the
//!   meaning of an existing value is a breaking change and requires a version
//!   bump.
//!
//! Optional fields are omitted rather than sent as `null` or as a placeholder
//! value; absent means "does not apply to this event", not "unknown".

mod config;
mod dispatch;
mod error;
mod idempotency;
mod job;
mod service;
mod signing;
mod types;

pub use config::WebhookConfig;
pub use dispatch::queue_for_store;
pub use error::WebhookError;
pub use idempotency::idempotency_key;
pub use job::WebhookJob;
pub use service::{WebhookDataService, WebhookService, WebhookSink};
pub use signing::sign_webhook_payload;
pub use types::{WEBHOOK_PAYLOAD_VERSION, WebhookEventType, WebhookPayload, WebhookPaymentInfo};
