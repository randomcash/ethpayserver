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

/// Queue `payload` for the store's configured webhook endpoint, if it has one.
///
/// Best-effort by design: a webhook is a notification, not a step in the
/// payment flow, so every failure here is logged and none propagate. Returns
/// `true` if a job was enqueued.
///
/// Skips silently when the store has no webhook, has it disabled, or has
/// turned this event off in `notification_prefs`.
#[allow(
    clippy::cognitive_complexity,
    reason = "the body is three linear steps; the tracing macros inflate the metric"
)]
pub async fn queue_for_store<D>(
    sink: &dyn WebhookSink,
    data_service: &D,
    store_id: Uuid,
    payload: WebhookPayload,
) -> bool
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
        return false;
    }

    let webhook_config = match data_service.get_enabled_webhook(store_id).await {
        Ok(Some(config)) => config,
        Ok(None) => {
            tracing::trace!(store_id = %store_id, "No webhook configured for store");
            return false;
        }
        Err(e) => {
            tracing::warn!(
                store_id = %store_id,
                error = %e,
                "Failed to get webhook config"
            );
            return false;
        }
    };

    let invoice_id = payload.invoice_id.clone();
    let job = WebhookJob::new(
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
        return false;
    }

    true
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
