//! Name/ownership gating and the payout/refund refusal for
//! `DELETE /admin/stores/{id}`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;

use auth::{Store, UserId};
use data_service::store_creation::StoreCreationWriter;
use server::api::admin::hard_delete_store;

use crate::support::*;

/// Half of `hard_delete_store`'s safety property: it hard-deletes on a live
/// server and cannot lean on "no financial history" the way
/// `delete_user_account` does, since removing a paid synthetic invoice is the
/// point. The name check is one of the two things that stand between it and a
/// real merchant's store - the other is ownership, covered by
/// `hard_delete_store_refuses_a_synthetic_name_owned_by_someone_else` below.
///
/// Owned by [`seed_e2e_owner`], not an arbitrary user: the handler's guard is
/// an `||` of the name check and the owner check, so pairing a non-synthetic
/// name with a non-E2E owner would pass on the owner check alone and prove
/// nothing about the name check actually being consulted.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refuses_a_name_that_is_not_the_synthetic_shape() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_e2e_owner(pg.pool()).await;
    let store = Store::new("A Real Merchant's Shop".to_string(), UserId(target));
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, _)) = result else {
        panic!("a non-synthetic store name must be refused");
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count store");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The other half: a store name is ordinary caller-supplied input, so
/// anyone who can create a store can give it the exact synthetic shape.
/// Without an ownership check, a real merchant's store that happened (or was
/// made) to collide on name - after it had taken a real payment but before
/// any payout or refund, the common early-life state of a store - would be
/// fully eligible for irreversible hard-deletion. This is the case that
/// makes the name check alone insufficient.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refuses_a_synthetic_name_owned_by_someone_else() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_user(pg.pool(), "user").await;
    let store = Store::new(
        "e2e-synthetic-2026-01-03T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    seed_payment(pg.pool(), &invoice).await;

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, _)) = result else {
        panic!(
            "a synthetic-named store owned by someone other than the E2E account must be refused"
        );
    };
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count store");
    assert_eq!(
        still_there, 1,
        "the refusal must not have deleted a real merchant's payment history"
    );

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The success path the synthetic-payment job's own cleanup and the store
/// backfill sweep both depend on: a matching-name store, along with the
/// invoice and payment it carries, is actually gone afterward - not merely
/// archived.
#[tokio::test]
#[ignore]
async fn hard_delete_store_removes_a_synthetic_store_with_its_invoice_and_payment() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_e2e_owner(pg.pool()).await;
    let store = Store::new(
        "e2e-synthetic-2026-01-01T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    seed_payment(pg.pool(), &invoice).await;

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    assert_eq!(result, Ok(StatusCode::NO_CONTENT));

    let stores: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count stores");
    assert_eq!(stores, 0);

    let invoices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE id = $1")
        .bind(&invoice)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count invoices");
    assert_eq!(
        invoices, 0,
        "the invoice should have gone with the store, not been left behind"
    );

    let payments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE invoice_id = $1")
        .bind(&invoice)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count payments");
    assert_eq!(payments, 0, "the payment should have cascaded away too");

    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// `ON DELETE CASCADE` does not reach `payouts` (see
/// `data_service::account_deletion`), so a store that somehow holds one -
/// which a synthetic E2E store never should - must be refused rather than
/// silently destroying it or failing halfway through the cascade.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refuses_when_a_payout_exists() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_e2e_owner(pg.pool()).await;
    let store = Store::new(
        "e2e-synthetic-2026-01-02T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    seed_payout(pg.pool(), store.id.0).await;

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, message)) = result else {
        panic!("a store holding a payout must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("payout"),
        "the operator must see why, got: {message}"
    );

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count store");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    clear_payouts_for_store(state.data_service.pool(), store.id.0).await;
    cleanup(state.data_service.pool(), &[caller, target]).await;
}

/// The other half of `ensure_no_payout_or_refund`'s `||`: the payout-only
/// seed above never exercises the `refund_count > 0` branch, so a typo'd
/// `&&`, or a refund reader pointed at the wrong store, would pass every test
/// in this file while a store holding a refund was still eligible for
/// destruction.
#[tokio::test]
#[ignore]
async fn hard_delete_store_refuses_when_a_refund_exists() {
    let Some(pg) = service().await else {
        return;
    };
    let caller = seed_user(pg.pool(), "server_admin").await;
    let target = seed_e2e_owner(pg.pool()).await;
    let store = Store::new(
        "e2e-synthetic-2026-01-04T00-00-00-000Z".to_string(),
        UserId(target),
    );
    pg.create_store_owned_by(&store, UserId(target))
        .await
        .expect("seed store");
    let invoice = seed_invoice(pg.pool(), store.id.0).await;
    let payment = seed_payment(pg.pool(), &invoice).await;
    seed_refund(pg.pool(), store.id.0, &invoice, payment).await;

    let state = app_state(Arc::new(pg));

    let result = hard_delete_store(
        admin_auth(caller),
        Path(store.id.0.to_string()),
        State(state.clone()),
    )
    .await;

    let Err((status, message)) = result else {
        panic!("a store holding a refund must be refused");
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        message.contains("refund"),
        "the operator must see why, got: {message}"
    );

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(state.data_service.pool())
        .await
        .expect("count store");
    assert_eq!(still_there, 1, "the refusal must not have deleted anything");

    clear_refunds_for_store(state.data_service.pool(), store.id.0).await;
    cleanup(state.data_service.pool(), &[caller, target]).await;
}
