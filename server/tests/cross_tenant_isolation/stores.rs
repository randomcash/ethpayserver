//! Stores: get-by-id and list, across tenants.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use server::api::AuthenticatedUser;

use crate::support::{app_state, seed_tenant, service, user_info};

#[tokio::test]
#[ignore]
async fn get_store_by_id_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::get_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to fetch B's store by id"
    );

    // Positive control: without this, an endpoint that refuses regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::stores::get_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to fetch A's own store by id");
    assert_eq!(own.id, a.store.id.0);
}

#[tokio::test]
#[ignore]
async fn list_stores_never_includes_another_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result =
        server::api::stores::list_stores(AuthenticatedUser(user_info(a.user_id)), State(state))
            .await
            .expect("listing one's own stores must succeed");

    let ids: Vec<Uuid> = result.iter().map(|s| s.id).collect();
    assert!(ids.contains(&a.store.id.0), "A's own store must be listed");
    assert!(
        !ids.contains(&b.store.id.0),
        "B's store leaked into A's store list"
    );
}
