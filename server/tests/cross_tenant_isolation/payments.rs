//! Payments: the same list/get shapes as invoices, kept in their own module
//! because each is its own handler with its own copy of the guard.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use uuid::Uuid;

use server::api::AuthenticatedUser;

use crate::support::{app_state, seed_tenant, service, status_of, user_info, user_info_with_role};

#[tokio::test]
#[ignore]
async fn list_payments_with_no_store_id_shows_only_the_callers_own() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: None,
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a merchant with no store filter must see their own payments, not be refused");

    let ids: Vec<String> = result.payments.iter().map(|p| p.id.clone()).collect();
    assert!(
        ids.contains(&a.payment_id.to_string()),
        "A's own payment must be visible with no store filter"
    );
    assert!(
        !ids.contains(&b.payment_id.to_string()),
        "B's payment leaked into A's unfiltered 'all stores' view"
    );
}

#[tokio::test]
#[ignore]
async fn list_payments_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(b.store.id.0),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "A must not be able to list B's payments by naming its store id directly"
    );

    // Positive control: without this, an endpoint that refuses every explicit
    // store_id, including the caller's own, would pass the assertion above
    // for the wrong reason.
    let own = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(a.store.id.0),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("A must be able to list A's own payments by naming its store id directly");
    let ids: Vec<String> = own.payments.iter().map(|p| p.id.clone()).collect();
    assert!(
        ids.contains(&a.payment_id.to_string()),
        "A's own payment must be visible when A names A's own store id"
    );
}

/// The payments side of the same nil-UUID bug the invoice test above guards
/// against: a nil `store_id` must be refused, not read as "every store".
#[tokio::test]
#[ignore]
async fn list_payments_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(Uuid::nil()),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "a nil store_id must not be treated as 'every store'"
    );
}

/// The payments side of `list_invoices_with_no_store_id_as_server_admin_sees_every_tenant`:
/// a `ServerAdmin` with no store filter must reach every tenant's payments,
/// not just their own.
#[tokio::test]
#[ignore]
async fn list_payments_with_no_store_id_as_server_admin_sees_every_tenant() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: None,
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a server admin must be able to list payments with no store filter");

    let ids: Vec<String> = result.payments.iter().map(|p| p.id.clone()).collect();
    assert!(
        ids.contains(&a.payment_id.to_string()),
        "an admin's unfiltered view must still include their own payment"
    );
    assert!(
        ids.contains(&b.payment_id.to_string()),
        "an admin's unfiltered view must reach every tenant, not just their own"
    );
}

#[tokio::test]
#[ignore]
async fn get_payment_by_id_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_payment(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.payment_id),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch B's payment by id"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::invoices::get_payment(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.payment_id),
    )
    .await
    .expect("A must be able to fetch A's own payment by id");
    assert_eq!(own.id, a.payment_id.to_string());
}
