//! Webhook delivery API endpoints.
//!
//! GET  /invoices/{invoice_id}/webhook-deliveries — recent deliveries for an invoice.
//! GET  /stores/{store_id}/webhook-deliveries — recent deliveries for a store.
//! POST /stores/{store_id}/webhook-deliveries/{delivery_id}/replay — resend one.
//!
//! `webhook_deliveries` used to be written by nobody: the retry queue existed,
//! backed off, and threw its history away the moment a job gave up. This is
//! the read side of making that history visible - scoped the same way
//! refunds and payouts already are, to the store the caller belongs to, 404
//! for anyone else's.

#[cfg(test)]
mod tests;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;
use uuid::Uuid;

use auth::{SessionService, UserStoreRepository};
use data_service::{InvoiceReader, StoreWebhookReader, WebhookDeliveryData, WebhookDeliveryReader};
use types::{InvoiceId, StoreId};

use super::extractors::AuthenticatedUser;
use crate::services::webhook::{WebhookJob, WebhookPayload};
use crate::state::PgAppState;

/// Pages are not exposed to the caller yet - the same fixed-size-page
/// starting point `payouts::list_payouts` used before pagination existed.
const DEFAULT_LIMIT: i64 = 50;

/// A delivery attempt as shown to a merchant.
///
/// `last_error` is whatever text the merchant's own endpoint, or the HTTP
/// client trying to reach it, produced - attacker-influenced from the
/// subscriber's point of view. It travels here as a plain JSON string field;
/// nothing in this crate renders it as HTML, and nothing downstream should.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookDeliveryResponse {
    pub id: Uuid,
    pub invoice_id: String,
    pub event_type: String,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<WebhookDeliveryData> for WebhookDeliveryResponse {
    fn from(d: WebhookDeliveryData) -> Self {
        Self {
            id: d.id,
            invoice_id: d.invoice_id,
            event_type: d.event_type,
            status: d.status.to_string(),
            attempts: d.attempts,
            max_attempts: d.max_attempts,
            last_error: d.last_error,
            created_at: d.created_at,
            updated_at: d.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookDeliveryListResponse {
    pub total: i64,
    pub deliveries: Vec<WebhookDeliveryResponse>,
}

/// Turn a stored payload into a fresh delivery of the same logical event.
///
/// Only `event_id` changes: it identifies one queued delivery (see the
/// `webhook` module docs), so a re-emission gets a new one. `idempotency_key`
/// is left untouched - it is what a subscriber dedupes on, so a replay that
/// changed it would look like a distinct event rather than a resend of the
/// one that was missed.
fn replayed_payload(mut original: WebhookPayload) -> WebhookPayload {
    original.event_id = Uuid::new_v4();
    original
}

/// Load a delivery, refusing one that belongs to a different store.
///
/// Mismatch answers 404, the same as a delivery id that does not exist at
/// all - matching `payouts::payout_for_store` and `refunds`, for the same
/// reason: the difference between "not yours" and "no such row" is itself a
/// fact about another merchant's data.
async fn delivery_for_store<R>(
    reader: &R,
    store_id: Uuid,
    delivery_id: Uuid,
) -> Result<WebhookDeliveryData, StatusCode>
where
    R: WebhookDeliveryReader,
{
    let delivery = WebhookDeliveryReader::get_delivery(reader, delivery_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if delivery.store_id != store_id {
        tracing::warn!(
            store_id = %store_id,
            "Webhook delivery lookup refused: the delivery belongs to another store"
        );
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(delivery)
}

/// List recent webhook deliveries for an invoice.
pub async fn list_deliveries_for_invoice<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<WebhookDeliveryListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, invoice.store_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let (total, deliveries) = WebhookDeliveryReader::list_deliveries_for_invoice(
        &*state.data_service,
        id.as_str(),
        DEFAULT_LIMIT,
        0,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(WebhookDeliveryListResponse {
        total,
        deliveries: deliveries.into_iter().map(Into::into).collect(),
    }))
}

/// List recent webhook deliveries for a store.
pub async fn list_deliveries_for_store<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(store_id): Path<Uuid>,
) -> Result<Json<WebhookDeliveryListResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, StoreId(store_id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let (total, deliveries) = WebhookDeliveryReader::list_deliveries_for_store(
        &*state.data_service,
        store_id,
        DEFAULT_LIMIT,
        0,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(WebhookDeliveryListResponse {
        total,
        deliveries: deliveries.into_iter().map(Into::into).collect(),
    }))
}

/// Replay a past delivery.
///
/// The stored payload is the exact one that was queued the first time, not a
/// snapshot of the invoice's current state, so it is sent verbatim except for
/// a fresh `event_id` - a replay is a new *delivery* of the same *event*.
/// `idempotency_key` is derived from the event's identity and is not touched,
/// so a subscriber that dedupes correctly sees no new event.
pub async fn replay_delivery<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path((store_id, delivery_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<WebhookDeliveryResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    if !user.role.is_admin()
        && state
            .data_service
            .get_user_store(user.id, StoreId(store_id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let delivery = delivery_for_store(&*state.data_service, store_id, delivery_id).await?;

    let original: WebhookPayload = serde_json::from_value(delivery.payload).map_err(|e| {
        tracing::error!(
            delivery_id = %delivery_id,
            error = %e,
            "Stored webhook payload failed to deserialize"
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let payload = replayed_payload(original);

    // The store's *current* webhook, not the one this delivery originally
    // went to: rotating the secret or URL must not strand old deliveries
    // behind a config that no longer exists, and a disabled webhook must not
    // be replayed into.
    let webhook_config = StoreWebhookReader::get_enabled_webhook(&*state.data_service, store_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::CONFLICT)?;

    let Some(sink) = &state.webhook_sink else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    let job = WebhookJob::new(
        webhook_config.id,
        webhook_config.webhook_url,
        webhook_config.webhook_secret,
        payload,
    );

    let response = WebhookDeliveryResponse {
        id: job.id,
        invoice_id: job.payload.invoice_id.clone(),
        event_type: job.payload.event_type.to_string(),
        status: "pending".to_string(),
        attempts: 0,
        max_attempts: job.max_attempts as i32,
        last_error: None,
        created_at: job.created_at,
        updated_at: job.created_at,
    };

    sink.queue(job).await.map_err(|e| {
        tracing::warn!(
            delivery_id = %delivery_id,
            error = %e,
            "Failed to queue webhook replay"
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(response))
}
