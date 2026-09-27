//! Store-scoped webhook dispatch.
//!
//! Every emitter — the event consumer, the expiry sweeper, the cancel
//! endpoint — needs the same three steps before a payload reaches the queue:
//! honour the store's notification preferences, look up its enabled webhook,
//! and enqueue. Three copies of that had already drifted (one checked
//! preferences, one did not), so it lives here once.

use types::{StoreSettingsReader, StoreWebhookReader};
use uuid::Uuid;

use super::{WebhookJob, WebhookPayload, WebhookSink};

/// What became of a `queue_for_store` call.
///
/// A plain `bool` return here used to conflate "nothing needed to happen"
/// with "something needed to happen and failed" - both were `false`, and
/// every caller discarded the value anyway, so the distinction had nowhere
/// to go. It matters to the webhook outbox drain: a `Skipped` obligation is
/// done (there was never anything to deliver), but a `Failed` one must be
/// retried on the next drain tick rather than marked dispatched, or the
/// obligation the outbox exists to protect is lost the same way the
/// synchronous call site used to lose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum QueueOutcome {
    /// Enqueued for delivery.
    Queued,
    /// Not an error: the store has no enabled webhook, or has turned this
    /// event off in `notification_prefs`. Nothing was ever going to be
    /// delivered.
    Skipped,
    /// Looking up the store's webhook, or handing the job to the sink,
    /// failed. The caller decides whether and how to retry.
    Failed,
}

impl QueueOutcome {
    /// Log a warning when queueing failed, identified by the fields a caller
    /// already has to hand. Every existing call site is fire-and-forget by
    /// design and does not act on the result beyond this - but "beyond this"
    /// used to mean "not at all", which is the gap a `bool` return with
    /// nowhere to go had left.
    pub fn warn_on_failure(self, invoice_id: &str, event: &str) {
        if let Self::Failed = self {
            tracing::warn!(invoice_id = %invoice_id, event = %event, "Failed to queue webhook");
        }
    }
}

/// Queue `payload` for the store's configured webhook endpoint, if it has one.
///
/// Skips (returns [`QueueOutcome::Skipped`]) when the store has no webhook,
/// has it disabled, or has turned this event off in `notification_prefs`.
#[allow(
    clippy::cognitive_complexity,
    reason = "the body is three linear steps; the tracing macros inflate the metric"
)]
pub async fn queue_for_store<D>(
    sink: &dyn WebhookSink,
    data_service: &D,
    store_id: Uuid,
    payload: WebhookPayload,
) -> QueueOutcome
where
    D: StoreWebhookReader + StoreSettingsReader + ?Sized,
{
    let event_key = payload.event_type.to_string();

    if suppressed_by_prefs(data_service, store_id, &event_key).await {
        tracing::trace!(
            store_id = %store_id,
            event = %event_key,
            "Webhook suppressed by notification_prefs"
        );
        return QueueOutcome::Skipped;
    }

    let webhook_config = match data_service.get_enabled_webhook(store_id).await {
        Ok(Some(config)) => config,
        Ok(None) => {
            tracing::trace!(store_id = %store_id, "No webhook configured for store");
            return QueueOutcome::Skipped;
        }
        Err(e) => {
            tracing::warn!(
                store_id = %store_id,
                error = %e,
                "Failed to get webhook config"
            );
            return QueueOutcome::Failed;
        }
    };

    let invoice_id = payload.invoice_id.clone();
    let job = WebhookJob::new(
        webhook_config.id,
        webhook_config.webhook_url,
        webhook_config.webhook_secret,
        payload,
    );

    if let Err(e) = sink.queue(job).await {
        tracing::warn!(
            invoice_id = %invoice_id,
            event = %event_key,
            error = %e,
            "Failed to queue webhook"
        );
        return QueueOutcome::Failed;
    }

    QueueOutcome::Queued
}

/// Has the store switched this event off for the webhook channel?
///
/// A store with no settings row, or no entry for this event, has not switched
/// anything off: absent means enabled.
async fn suppressed_by_prefs<D>(data_service: &D, store_id: Uuid, event_key: &str) -> bool
where
    D: StoreSettingsReader + ?Sized,
{
    let Ok(Some(settings)) = data_service.get_store_settings(store_id).await else {
        return false;
    };

    settings
        .notification_prefs
        .get(event_key)
        .and_then(|prefs| prefs.get("webhook"))
        == Some(&serde_json::Value::Bool(false))
}
