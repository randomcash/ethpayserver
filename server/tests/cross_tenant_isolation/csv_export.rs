//! CSV export: builds its filter through the same `verify_store_access_for_query`
//! guard as `list_invoices`/`list_payments`, but is a separate handler and a
//! separate response path (a streamed file, not JSON), so the guard being
//! wired to the list endpoint proves nothing about the export one.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use uuid::Uuid;

use server::api::AuthenticatedUser;

use crate::support::{
    app_state, authenticate_via_bearer, seed_tenant, service, status_of, user_info,
};

#[tokio::test]
#[ignore]
async fn export_invoices_csv_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_invoices_csv(
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
        "A must not be able to export B's store's invoices by naming its id directly"
    );

    // Positive control: without this, an endpoint that refuses every explicit
    // store_id, including the caller's own, would pass the assertion above
    // for the wrong reason.
    let own = server::api::invoices::export_invoices_csv(
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
    .expect("A must be able to export A's own store's invoices by naming its id directly");
    assert_eq!(own.status(), StatusCode::OK);
}

/// The same nil-UUID regression `list_invoices` guards against, but for the
/// export handler's own copy of the store-access check.
#[tokio::test]
#[ignore]
async fn export_invoices_csv_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_invoices_csv(
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

#[tokio::test]
#[ignore]
async fn export_payments_csv_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_payments_csv(
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
        "A must not be able to export B's store's payments by naming its id directly"
    );

    // Positive control, same reasoning as the invoice export above.
    let own = server::api::invoices::export_payments_csv(
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
    .expect("A must be able to export A's own store's payments by naming its id directly");
    assert_eq!(own.status(), StatusCode::OK);
}

#[tokio::test]
#[ignore]
async fn export_payments_csv_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_payments_csv(
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

/// Both export handlers take the same `AuthenticatedUser` extractor as every
/// other endpoint in this suite, so an API key reaches them the same way a
/// session does - this is the export side of the same "carries less than its
/// owner's scope" check `api_keys::cross_tenant_reads` runs for other
/// resources.
#[tokio::test]
#[ignore]
async fn csv_export_via_api_key_cannot_reach_another_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::invoices::export_invoices_csv(
        a_via_key,
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
        "an API key must not export B's store's invoices by naming its id directly"
    );

    // Positive control: without this, an endpoint that refuses every API-key
    // caller, including the owner's own store, would pass the assertion above
    // for the wrong reason.
    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own = server::api::invoices::export_invoices_csv(
        a_via_key,
        State(state.clone()),
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
    .expect("an API key must be able to export its owner's own store's invoices");
    assert_eq!(own.status(), StatusCode::OK);

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::invoices::export_payments_csv(
        a_via_key,
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
        "an API key must not export B's store's payments by naming its id directly"
    );

    // Positive control, same reasoning as above.
    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own = server::api::invoices::export_payments_csv(
        a_via_key,
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
    .expect("an API key must be able to export its owner's own store's payments");
    assert_eq!(own.status(), StatusCode::OK);
}
