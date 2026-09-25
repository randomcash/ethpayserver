//! Invoices: list (unfiltered, by store id, by nil store id, as admin) and
//! get-by-id, payments-on-invoice, and status, all across tenants.
//!
//! Review finding, checked: `list_invoices`, `get_invoice`,
//! `get_invoice_payments` and `get_invoice_status` are mounted at
//! `GET /invoices/`, `GET /invoices/{invoice_id}`,
//! `GET /invoices/{invoice_id}/payments` and
//! `GET /invoices/{invoice_id}/status` (`server/src/api/mod.rs`) - not
//! orphaned handlers.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use uuid::Uuid;

use server::api::AuthenticatedUser;

use crate::support::{app_state, seed_tenant, service, status_of, user_info, user_info_with_role};

#[tokio::test]
#[ignore]
async fn list_invoices_with_no_store_id_shows_only_the_callers_own() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: None,
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a merchant with no store filter must see their own invoices, not be refused");

    let ids: Vec<String> = result.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(
        ids.contains(&a.invoice.id.0),
        "A's own invoice must be visible with no store filter"
    );
    assert!(
        !ids.contains(&b.invoice.id.0),
        "B's invoice leaked into A's unfiltered 'all stores' view"
    );
}

#[tokio::test]
#[ignore]
async fn list_invoices_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(b.store.id.0),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "A must not be able to list B's store by naming its id directly"
    );

    // Positive control: without this, an endpoint that refuses every explicit
    // store_id, including the caller's own, would pass the assertion above
    // for the wrong reason.
    let own = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(a.store.id.0),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("A must be able to list A's own store by naming its id directly");
    let ids: Vec<String> = own.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(
        ids.contains(&a.invoice.id.0),
        "A's own invoice must be visible when A names A's own store id"
    );
}

/// The literal historical bug: a nil UUID once took a different code path
/// than "no filter" or "a real foreign store id" and skipped both the
/// membership check and the `WHERE store_id` clause, handing a merchant
/// every invoice on the server. A nil `store_id` must be refused exactly
/// like any other store A does not belong to, not treated as "everything".
#[tokio::test]
#[ignore]
async fn list_invoices_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(Uuid::nil()),
            status: None,
            currency: None,
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

/// Contrasts the two membership-only tests above: a `ServerAdmin` asking for
/// the same store filter as a `User` must not stop at the caller's own
/// stores. If this bypass ever silently loosened to cover `Role::User` too,
/// the earlier tests would already fail; this test is what proves the
/// bypass is real for the role that is supposed to have it.
#[tokio::test]
#[ignore]
async fn list_invoices_with_no_store_id_as_server_admin_sees_every_tenant() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: None,
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a server admin must be able to list with no store filter");

    let ids: Vec<String> = result.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(
        ids.contains(&a.invoice.id.0),
        "an admin's unfiltered view must still include their own invoice"
    );
    assert!(
        ids.contains(&b.invoice.id.0),
        "an admin's unfiltered view must reach every tenant, not just their own"
    );
}

#[tokio::test]
#[ignore]
async fn get_invoice_by_id_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to fetch B's invoice by id"
    );

    // Positive control: the admin test below proves the admin bypass works,
    // but says nothing about the ownership branch a regular merchant goes
    // through. Without this, an endpoint that refused every non-admin caller
    // regardless of ownership would still pass the assertion above for the
    // wrong reason.
    let own = server::api::invoices::get_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to fetch A's own invoice by id");
    assert_eq!(own.id, a.invoice.id.0);
}

/// The positive control for the test above: the same cross-tenant request,
/// with the caller's role swapped to `ServerAdmin`, must succeed. Without
/// this, a bug that made every caller FORBIDDEN regardless of role would
/// still pass the negative test.
#[tokio::test]
#[ignore]
async fn get_invoice_across_tenants_is_permitted_for_a_server_admin() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.invoice.id.0.clone()),
    )
    .await
    .expect("a server admin must be able to fetch any tenant's invoice by id");

    assert_eq!(result.id, b.invoice.id.0);
}

#[tokio::test]
#[ignore]
async fn get_invoice_payments_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list B's invoice's payments"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::invoices::get_invoice_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to list payments on A's own invoice");
    assert!(own.iter().any(|p| p.id == a.payment_id.to_string()));
}

#[tokio::test]
#[ignore]
async fn get_invoice_status_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice_status(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to read B's invoice status"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::invoices::get_invoice_status(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to read A's own invoice status");
    assert_eq!(own.id, a.invoice.id.0);
}
