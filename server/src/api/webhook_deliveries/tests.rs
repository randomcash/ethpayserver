#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Authorization-boundary and replay-identity tests for webhook deliveries.
//!
//! Two properties are pinned here:
//!
//! 1. A delivery is read back only by the store that owns it. The store id in
//!    the path is checked by the membership gate, but the delivery itself has
//!    to be matched against it too, or one store's webhook history (which
//!    includes `last_error`, attacker-influenced text from the merchant's own
//!    endpoint) leaks to another. The refusal is 404, never 403, matching
//!    `payouts::payout_for_store`.
//! 2. A replay resends the same logical event: `idempotency_key` is
//!    unchanged, only `event_id` (which identifies one delivery, not one
//!    event) is fresh - so a subscriber that dedupes on the key correctly
//!    sees no new event.

use super::{delivery_for_store, replayed_payload};

use async_trait::async_trait;
use chrono::Utc;
use data_service::{
    RepositoryResult, WebhookDeliveryData, WebhookDeliveryReader, WebhookDeliveryStatus,
};
use types::{InvoiceData, InvoiceId, InvoiceStatus, StoreId};
use uuid::Uuid;

use crate::services::webhook::{WebhookEventType, WebhookPayload};

fn test_invoice() -> InvoiceData {
    InvoiceData {
        id: InvoiceId::from_string("inv_1".to_string()),
        store_id: StoreId::new(),
        currency: "ETH".to_string(),
        status: InvoiceStatus::Paid,
        amount: "1000".to_string(),
        amount_received: "1000".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    }
}

fn test_payload() -> WebhookPayload {
    WebhookPayload::invoice_event(WebhookEventType::InvoiceExpired, &test_invoice())
}

fn delivery(store_id: Uuid) -> WebhookDeliveryData {
    WebhookDeliveryData {
        id: Uuid::new_v4(),
        store_webhook_id: Uuid::new_v4(),
        store_id,
        invoice_id: "inv_1".to_string(),
        event_type: "invoice_expired".to_string(),
        status: WebhookDeliveryStatus::Delivered,
        attempts: 1,
        max_attempts: 7,
        last_error: None,
        payload: serde_json::to_value(test_payload()).unwrap(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// A single fixed delivery, standing in for the database.
struct StubReader(WebhookDeliveryData);

#[async_trait]
impl WebhookDeliveryReader for StubReader {
    async fn get_delivery(&self, id: Uuid) -> RepositoryResult<Option<WebhookDeliveryData>> {
        Ok((self.0.id == id).then(|| self.0.clone()))
    }

    async fn list_deliveries_for_invoice(
        &self,
        _invoice_id: &str,
        _limit: i64,
        _offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDeliveryData>)> {
        unimplemented!("not exercised by the store-scoping gate")
    }

    async fn list_deliveries_for_store(
        &self,
        _store_id: Uuid,
        _limit: i64,
        _offset: i64,
    ) -> RepositoryResult<(i64, Vec<WebhookDeliveryData>)> {
        unimplemented!("not exercised by the store-scoping gate")
    }
}

#[tokio::test]
async fn a_delivery_is_readable_by_the_store_that_owns_it() {
    let mine = Uuid::new_v4();
    let reader = StubReader(delivery(mine));

    let found = delivery_for_store(&reader, mine, reader.0.id).await;
    assert!(found.is_ok());
}

#[tokio::test]
async fn another_stores_delivery_is_refused_with_404_not_403() {
    let mine = Uuid::new_v4();
    let theirs_delivery = delivery(Uuid::new_v4());
    let reader = StubReader(theirs_delivery);

    let err = delivery_for_store(&reader, mine, reader.0.id)
        .await
        .expect_err("a delivery belonging to another store must be refused");

    assert_eq!(
        err,
        axum::http::StatusCode::NOT_FOUND,
        "the refusal must be 404, not 403: the difference between \"not \
         yours\" and \"no such row\" is itself a fact about another \
         merchant's data"
    );
}

#[tokio::test]
async fn an_unknown_delivery_id_is_also_404() {
    let mine = Uuid::new_v4();
    let reader = StubReader(delivery(mine));

    let err = delivery_for_store(&reader, mine, Uuid::new_v4())
        .await
        .expect_err("an unknown id must be refused");

    assert_eq!(err, axum::http::StatusCode::NOT_FOUND);
}

/// The property replay rests on: build it twice (original, then replayed),
/// get one key.
#[test]
fn replay_keeps_the_idempotency_key_but_not_the_event_id() {
    let original = test_payload();
    let replayed = replayed_payload(original.clone());

    assert_eq!(
        replayed.idempotency_key, original.idempotency_key,
        "a subscriber dedupes on this key; changing it would make a replay \
         look like a distinct event instead of a resend"
    );
    assert_ne!(
        replayed.event_id, original.event_id,
        "event_id identifies one delivery, and a replay is a new one"
    );
}
