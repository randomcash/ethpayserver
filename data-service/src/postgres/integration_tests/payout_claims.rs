//! Which invoices a store's payouts already claim, asked of a real database.
//!
//! A payout's invoice ids live in a JSONB array, and the question "is this
//! invoice already claimed?" is asked of every payout the store has. Nothing
//! about that is exercisable against a mock: the unnesting, the status filter
//! and the store scope are all SQL. So this runs against Postgres.
//!
//! The three properties that matter, and each has money behind it:
//! - an invoice claimed by a live payout is reported, so it cannot be claimed
//!   twice;
//! - a failed payout releases its invoices, because nothing moved;
//! - the answer never reaches past the store that asked.

use chrono::Utc;
use sqlx::PgPool;
use types::{ChainId, PayoutData, PayoutStatus, StoreId};
use uuid::Uuid;

use crate::postgres::PgDataService;
use crate::{PayoutClaimReader, PayoutWriter};

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

async fn seed_payout(
    service: &PgDataService,
    store_id: StoreId,
    invoice_ids: &[&str],
    status: PayoutStatus,
) {
    let payout = PayoutData {
        id: Uuid::new_v4(),
        store_id,
        invoice_ids: invoice_ids.iter().map(|s| (*s).to_string()).collect(),
        destination_address: "0xmerchant".to_string(),
        chain_id: ChainId::evm(1),
        asset_type: "native".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        amount: "1000".to_string(),
        tx_hash: None,
        status,
        fee_amount: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    };

    service.create_payout(&payout).await.expect("seed payout");
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_string()).collect()
}

#[tokio::test]
#[ignore]
async fn a_live_payout_holds_the_invoices_it_names() {
    let Some(service) = service().await else {
        return;
    };
    let store = seed_store(&service.pool).await;
    let claimed_id = format!("inv-claimed-{}", Uuid::new_v4());
    let free_id = format!("inv-free-{}", Uuid::new_v4());

    seed_payout(
        &service,
        store,
        &[&claimed_id, "inv-other"],
        PayoutStatus::Pending,
    )
    .await;

    let claimed = service
        .invoice_ids_already_claimed(store, &ids(&[&claimed_id, &free_id]))
        .await
        .expect("read claims");

    assert_eq!(
        claimed,
        vec![claimed_id],
        "an invoice inside a pending payout's array is already claimed; one \
         nobody named is not"
    );
}

#[tokio::test]
#[ignore]
async fn a_failed_payout_releases_its_invoices() {
    let Some(service) = service().await else {
        return;
    };
    let store = seed_store(&service.pool).await;
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    seed_payout(&service, store, &[&invoice_id], PayoutStatus::Failed).await;

    let claimed = service
        .invoice_ids_already_claimed(store, &ids(&[&invoice_id]))
        .await
        .expect("read claims");

    assert!(
        claimed.is_empty(),
        "a failed payout moved no money, so its invoices are claimable again"
    );
}

#[tokio::test]
#[ignore]
async fn confirmed_and_broadcasting_payouts_hold_theirs() {
    let Some(service) = service().await else {
        return;
    };
    let store = seed_store(&service.pool).await;
    let confirmed_id = format!("inv-confirmed-{}", Uuid::new_v4());
    let broadcasting_id = format!("inv-broadcasting-{}", Uuid::new_v4());

    seed_payout(&service, store, &[&confirmed_id], PayoutStatus::Confirmed).await;
    seed_payout(
        &service,
        store,
        &[&broadcasting_id],
        PayoutStatus::Broadcasting,
    )
    .await;

    let mut claimed = service
        .invoice_ids_already_claimed(store, &ids(&[&confirmed_id, &broadcasting_id]))
        .await
        .expect("read claims");
    claimed.sort();

    let mut expected = vec![confirmed_id, broadcasting_id];
    expected.sort();
    assert_eq!(claimed, expected);
}

#[tokio::test]
#[ignore]
async fn another_stores_claim_is_not_reported() {
    let Some(service) = service().await else {
        return;
    };
    let mine = seed_store(&service.pool).await;
    let theirs = seed_store(&service.pool).await;
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    seed_payout(&service, theirs, &[&invoice_id], PayoutStatus::Pending).await;

    let claimed = service
        .invoice_ids_already_claimed(mine, &ids(&[&invoice_id]))
        .await
        .expect("read claims");

    assert!(
        claimed.is_empty(),
        "the answer is about this store's payouts; another store's are not \
         this caller's to be told about"
    );
}

#[tokio::test]
#[ignore]
async fn nothing_asked_is_nothing_claimed() {
    let Some(service) = service().await else {
        return;
    };
    let store = seed_store(&service.pool).await;

    let claimed = service
        .invoice_ids_already_claimed(store, &[])
        .await
        .expect("read claims");

    assert!(claimed.is_empty());
}
