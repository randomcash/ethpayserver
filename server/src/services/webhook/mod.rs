//\! Webhook notification service.
//\!
//\! Delivers webhook notifications to merchants when invoice status changes.
//\! Uses Redis as a job queue with exponential backoff retries.

mod config;
mod error;
mod job;
mod service;
mod signing;
mod types;

pub use config::WebhookConfig;
pub use error::WebhookError;
pub use job::WebhookJob;
pub use service::{WebhookDataService, WebhookService};
pub use signing::sign_webhook_payload;
pub use types::{WebhookEventType, WebhookPayload, WebhookPaymentInfo};
