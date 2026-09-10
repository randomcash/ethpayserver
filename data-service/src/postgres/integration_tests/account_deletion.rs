//! What blocks deleting an account, against a real database.
//!
//! The counts matter more than they look. `users` cascades through `stores`
//! into `invoices` and `payments`, so a query that misses a row lets the
//! endpoint delete a merchant's financial history; one that counts a row it
//! should not refuses a deletion that was safe. Both are only visible against
//! the real foreign keys, which is why these are integration tests.

use sqlx::{Executor, PgPool};
use uuid::Uuid;

use crate::account_deletion::AccountDeletionReader;
use crate::postgres::PgDataService;

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_store(pool: &PgPool, owner: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(format!("store-{id}"))
        .bind(owner)
        .execute(pool)
        .await
        .expect("seed store");
    id
}

async fn seed_invoice(pool: &PgPool, store: Uuid) -> String {
    let id = format!("inv-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO invoices (id, store_id, currency, amount, expires_at) \
         VALUES ($1, $2, 'USD', 10, now() + interval '1 hour')",
    )
    .bind(&id)
    .bind(store)
    .execute(pool)
    .await
    .expect("seed invoice");
    id
}

async fn seed_payment(pool: &PgPool, invoice: &str) {
    sqlx::query(
        "INSERT INTO payments (invoice_id, chain_id, asset_type, asset_symbol, amount, tx_hash) \
         VALUES ($1, 'eip155:11155111', 'native', 'ETH', 1, $2)",
    )
    .bind(invoice)
    .bind(format!("0x{}", Uuid::new_v4().simple()))
    .execute(pool)
    .await
    .expect("seed payment");
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    let _ = pool
        .execute(sqlx::query("DELETE FROM users WHERE id = $1").bind(user))
        .await;
}

#[tokio::test]
#[ignore]
async fn an_account_that_never_traded_has_nothing_blocking_it() {
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = seed_store(&service.pool, user).await;
    // An invoice alone is not money: nobody paid it, so nothing is lost by
    // deleting it. Only payments, payouts and refunds block.
    seed_invoice(&service.pool, store).await;

    let blockers = service
        .account_deletion_blockers(auth::UserId(user))
        .await
        .expect("read blockers");

    assert_eq!(blockers.payments, 0);
    assert_eq!(blockers.payouts, 0);
    assert_eq!(blockers.refunds, 0);
    assert!(!blockers.any(), "an unpaid invoice must not block deletion");

    cleanup(&service.pool, user).await;
}

#[tokio::test]
#[ignore]
async fn a_payment_blocks_deletion() {
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = seed_store(&service.pool, user).await;
    let invoice = seed_invoice(&service.pool, store).await;
    seed_payment(&service.pool, &invoice).await;

    let blockers = service
        .account_deletion_blockers(auth::UserId(user))
        .await
        .expect("read blockers");

    assert_eq!(blockers.payments, 1, "the payment must be counted");
    assert!(blockers.any());
    assert!(blockers.describe().contains("1 payment(s)"));

    cleanup(&service.pool, user).await;
}

#[tokio::test]
#[ignore]
async fn another_merchants_payments_do_not_block_me() {
    // The count walks `stores.owner_id`, which is the column that cascades.
    // Counting anything wider would refuse a deletion that is perfectly safe.
    let Some(service) = service().await else {
        return;
    };
    let me = seed_user(&service.pool).await;
    let them = seed_user(&service.pool).await;
    seed_store(&service.pool, me).await;
    let their_store = seed_store(&service.pool, them).await;
    let their_invoice = seed_invoice(&service.pool, their_store).await;
    seed_payment(&service.pool, &their_invoice).await;

    let mine = service
        .account_deletion_blockers(auth::UserId(me))
        .await
        .expect("read blockers");
    let theirs = service
        .account_deletion_blockers(auth::UserId(them))
        .await
        .expect("read blockers");

    assert!(!mine.any(), "another account's payment must not block mine");
    assert_eq!(theirs.payments, 1);

    cleanup(&service.pool, me).await;
    cleanup(&service.pool, them).await;
}

#[tokio::test]
#[ignore]
async fn deleting_an_untraded_account_takes_its_stores_with_it() {
    // The cascade is the point: what deletion is for is leaving nothing behind.
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = seed_store(&service.pool, user).await;

    auth::UserRepository::delete_user(&service, auth::UserId(user))
        .await
        .expect("delete user");

    let stores: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store)
        .fetch_one(&service.pool)
        .await
        .expect("count stores");
    assert_eq!(stores, 0, "the store should have gone with the account");

    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(&service.pool)
        .await
        .expect("count users");
    assert_eq!(users, 0);
}

#[tokio::test]
#[ignore]
async fn without_the_guard_a_delete_destroys_the_payment_history() {
    // The reason the guard exists, pinned. `delete_user` is the raw cascade with
    // nothing in front of it, and this is what it does to a merchant who traded:
    // the invoice and the payment go with the account, silently and with no
    // error. Nothing in the schema stops it, which is why the refusal has to
    // live in the endpoint - and why anyone reaching for `delete_user` directly
    // needs to have read this.
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = seed_store(&service.pool, user).await;
    let invoice = seed_invoice(&service.pool, store).await;
    seed_payment(&service.pool, &invoice).await;

    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE invoice_id = $1")
        .bind(&invoice)
        .fetch_one(&service.pool)
        .await
        .expect("count before");
    assert_eq!(before, 1);

    auth::UserRepository::delete_user(&service, auth::UserId(user))
        .await
        .expect("the raw delete succeeds - that is the problem");

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE invoice_id = $1")
        .bind(&invoice)
        .fetch_one(&service.pool)
        .await
        .expect("count after");
    assert_eq!(
        after, 0,
        "if this ever fails the cascade changed, and the endpoint's refusal \
         should be revisited alongside it"
    );
}
