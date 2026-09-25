//! Payouts, refunds, and webhook deliveries: the same store-membership shape
//! as invoices and payments (a path id checked with `get_user_store`, then the
//! row itself matched to that store), so the same nil/foreign/admin questions
//! apply and had no coverage at all before this test.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use server::api::AuthenticatedUser;

use crate::support::{
    app_state, seed_payout, seed_refund, seed_tenant, seed_webhook_delivery, service, user_info,
    user_info_with_role,
};

#[tokio::test]
#[ignore]
async fn payout_endpoints_refuse_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_payout = seed_payout(&pg, &a.store).await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path((b.store.id.0, b_payout)),
    )
    .await;
    assert_eq!(
        get_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch a payout on B's store"
    );

    let list_result = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        list_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list payouts on B's store"
    );

    // Positive control: without this, both endpoints refusing every caller,
    // including one asking about their own store, would pass the assertions
    // above for the wrong reason.
    let own = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path((a.store.id.0, a_payout)),
    )
    .await
    .expect("A must be able to fetch a payout on A's own store");
    assert_eq!(own.id, a_payout);

    let own_list = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to list payouts on A's own store");
    assert!(own_list.payouts.iter().any(|p| p.id == a_payout));
}

/// The "id from B passed directly to a detail endpoint" case: A names A's own
/// store, so the membership gate passes, but supplies B's payout id. The
/// membership check alone must not be enough - the payout itself has to be
/// matched to the store named in the path, the same as every other
/// `*_for_store` lookup in this file.
#[tokio::test]
#[ignore]
async fn get_payout_refuses_another_tenants_payout_even_via_the_callers_own_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path((a.store.id.0, b_payout)),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch B's payout by naming A's own store and B's payout id"
    );
}

#[tokio::test]
#[ignore]
async fn list_refunds_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_refund = seed_refund(&pg, &a).await;
    let _b_refund = seed_refund(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::refunds::list_refunds(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list refunds on B's invoice"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::refunds::list_refunds(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to list refunds on A's own invoice");
    assert!(own.iter().any(|r| r.id == a_refund));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_invoice_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list webhook deliveries on B's invoice"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::webhook_deliveries::list_deliveries_for_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to list webhook deliveries on A's own invoice");
    assert!(own.deliveries.iter().any(|d| d.id == a_delivery));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_store_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list webhook deliveries on B's store"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to list webhook deliveries on A's own store");
    assert!(own.deliveries.iter().any(|d| d.id == a_delivery));
}

/// The nil-`store_id` bug shape, for the two endpoints in this section keyed
/// directly by a `store_id` path segment. `list_refunds` and
/// `list_deliveries_for_invoice` are keyed by invoice id (a string, not a
/// UUID) instead, so there is no nil-`store_id` case to construct for them -
/// the admin-bypass tests below cover those two.
#[tokio::test]
#[ignore]
async fn payout_endpoints_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let a_payout = seed_payout(&pg, &a.store).await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path((Uuid::nil(), a_payout)),
    )
    .await;
    assert_eq!(
        get_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a nil store_id must not be treated as 'every store'"
    );

    let list_result = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(Uuid::nil()),
    )
    .await;
    assert_eq!(
        list_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a nil store_id must not be treated as 'every store'"
    );
}

/// The admin-bypass side of the same shape: a `ServerAdmin` is not a member
/// of either tenant's store, yet must still reach both - the direction where
/// an over-narrow membership check would wrongly refuse the one role that is
/// supposed to see everything.
#[tokio::test]
#[ignore]
async fn payout_endpoints_as_server_admin_reach_every_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state.clone()),
        Path((b.store.id.0, b_payout)),
    )
    .await
    .expect("a server admin must be able to fetch another tenant's payout");
    assert_eq!(get_result.id, b_payout);

    let list_result = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.store.id.0),
    )
    .await
    .expect("a server admin must be able to list another tenant's payouts");
    assert!(list_result.payouts.iter().any(|p| p.id == b_payout));
}

#[tokio::test]
#[ignore]
async fn list_refunds_as_server_admin_reaches_another_tenants_invoice() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_refund = seed_refund(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::refunds::list_refunds(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.invoice.id.0.clone()),
    )
    .await
    .expect("a server admin must be able to list another tenant's refunds");
    assert!(result.iter().any(|r| r.id == b_refund));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_invoice_as_server_admin_reaches_another_tenants_invoice() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_invoice(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.invoice.id.0.clone()),
    )
    .await
    .expect("a server admin must be able to list another tenant's webhook deliveries");
    assert!(result.deliveries.iter().any(|d| d.id == b_delivery));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_store_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(Uuid::nil()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a nil store_id must not be treated as 'every store'"
    );
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_store_as_server_admin_reaches_every_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.store.id.0),
    )
    .await
    .expect("a server admin must be able to list another tenant's webhook deliveries by store");
    assert!(result.deliveries.iter().any(|d| d.id == b_delivery));
}
