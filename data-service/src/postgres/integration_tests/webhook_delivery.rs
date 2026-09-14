//! `webhook_deliveries`, asked of a real database.
//!
//! Two properties matter enough to run against Postgres rather than a mock:
//! - the `ON CONFLICT (id)` upsert is what makes retries of the same job
//!   collapse to one row instead of accumulating one per attempt, and that is
//!   exactly the kind of thing a mock would get right by construction whether
//!   or not the real SQL does;
//! - `list_deliveries_for_store` joins through `store_webhooks` for a column
//!   the delivery row itself does not carry, so the store scope is SQL, not
//!   Rust.

use sqlx::PgPool;
use types::{StoreId, StoreWebhookWriter};
use uuid::Uuid;

use crate::postgres::PgDataService;
use crate::{
    UpsertDeliveryParams, WebhookDeliveryReader, WebhookDeliveryStatus, WebhookDeliveryWriter,
};

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

async fn seed_store(pool: &PgPool) -> StoreId {
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed user");

    let store_id = Uuid::new_v4();
    sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(store_id)
        .bind(format!("store-{store_id}"))
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed store");

    StoreId(store_id)
}

/// A store with a webhook configured, returning the webhook's own id -
/// what a delivery row's `store_webhook_id` points at.
async fn seed_store_webhook(service: &PgDataService, store_id: StoreId) -> Uuid {
    let webhook = service
        .upsert_webhook(store_id.0, "https://example.com/webhook", "secret", true)
        .await
        .expect("seed store webhook");
    webhook.id
}

fn params(
    id: Uuid,
    store_webhook_id: Uuid,
    status: WebhookDeliveryStatus,
    attempts: i32,
    last_error: Option<&str>,
) -> UpsertDeliveryParams {
    UpsertDeliveryParams {
        id,
        store_webhook_id,
        invoice_id: "inv_1".to_string(),
        event_type: "invoice_expired".to_string(),
        status,
        attempts,
        max_attempts: 7,
        last_error: last_error.map(str::to_string),
        payload: serde_json::json!({"event_type": "invoice_expired"}),
    }
}

#[tokio::test]
#[ignore]
async fn two_failures_then_a_success_leave_one_row_not_three() {
    let Some(service) = service().await else {
        return;
    };
    let store = seed_store(&service.pool).await;
    let store_webhook_id = seed_store_webhook(&service, store).await;
    let job_id = Uuid::new_v4();

    // Queued.
    service
        .upsert_delivery(params(
            job_id,
            store_webhook_id,
            WebhookDeliveryStatus::Pending,
            0,
            None,
        ))
        .await
        .expect("insert pending");
    // Attempt 1 fails, scheduled for retry.
    service
        .upsert_delivery(params(
            job_id,
            store_webhook_id,
            WebhookDeliveryStatus::Retrying,
            1,
            Some("HTTP 500"),
        ))
        .await
        .expect("record attempt 1");
    // Attempt 2 fails, scheduled for retry.
    service
        .upsert_delivery(params(
            job_id,
            store_webhook_id,
            WebhookDeliveryStatus::Retrying,
            2,
            Some("HTTP 500"),
        ))
        .await
        .expect("record attempt 2");
    // Attempt 3 succeeds.
    service
        .upsert_delivery(params(
            job_id,
            store_webhook_id,
            WebhookDeliveryStatus::Delivered,
            3,
            None,
        ))
        .await
        .expect("record attempt 3");

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM webhook_deliveries WHERE id = $1")
        .bind(job_id)
        .fetch_one(&service.pool)
        .await
        .expect("count rows");
    assert_eq!(
        count.0, 1,
        "four writes for the same job id must leave one row, not one per write"
    );

    let delivery = service
        .get_delivery(job_id)
        .await
        .expect("read delivery")
        .expect("delivery exists");
    assert_eq!(delivery.attempts, 3);
    assert_eq!(delivery.status, WebhookDeliveryStatus::Delivered);
    assert!(
        delivery.last_error.is_none(),
        "the row reflects the final attempt, not an earlier failed one"
    );
}

#[tokio::test]
#[ignore]
async fn another_stores_delivery_is_not_listed() {
    let Some(service) = service().await else {
        return;
    };
    let mine = seed_store(&service.pool).await;
    let theirs = seed_store(&service.pool).await;
    let my_webhook = seed_store_webhook(&service, mine).await;
    let their_webhook = seed_store_webhook(&service, theirs).await;

    service
        .upsert_delivery(params(
            Uuid::new_v4(),
            my_webhook,
            WebhookDeliveryStatus::Delivered,
            1,
            None,
        ))
        .await
        .expect("seed my delivery");
    service
        .upsert_delivery(params(
            Uuid::new_v4(),
            their_webhook,
            WebhookDeliveryStatus::Delivered,
            1,
            None,
        ))
        .await
        .expect("seed their delivery");

    let (total, deliveries) = service
        .list_deliveries_for_store(mine.0, 50, 0)
        .await
        .expect("list my deliveries");

    assert_eq!(total, 1);
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].store_webhook_id, my_webhook);
    assert_eq!(
        deliveries[0].store_id, mine.0,
        "the joined store_id must be the caller's own store"
    );
}

#[tokio::test]
#[ignore]
async fn get_delivery_reports_the_owning_store() {
    let Some(service) = service().await else {
        return;
    };
    let store = seed_store(&service.pool).await;
    let store_webhook_id = seed_store_webhook(&service, store).await;
    let job_id = Uuid::new_v4();

    service
        .upsert_delivery(params(
            job_id,
            store_webhook_id,
            WebhookDeliveryStatus::Pending,
            0,
            None,
        ))
        .await
        .expect("seed delivery");

    let delivery = service
        .get_delivery(job_id)
        .await
        .expect("read delivery")
        .expect("delivery exists");
    assert_eq!(delivery.store_id, store.0);
}
