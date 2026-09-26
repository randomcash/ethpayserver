//! Store members, webhook config and token policy: the same
//! `require_store_settings_permission`/permission-gated `Path<Uuid>` shape as
//! the store wallet endpoints, tested separately because each is its own
//! handler with its own copy of the guard.
//!
//! Review finding, checked: `list_store_members`, `get_store_webhook` and
//! `get_token_policy` are mounted at `GET /{store_id}/members`,
//! `GET /{store_id}/webhook` and `GET /{store_id}/token-policy`
//! (`server/src/api/mod.rs`) - not orphaned handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;

use server::api::AuthenticatedUser;

use crate::support::{
    app_state, authenticate_via_bearer, grant_store_permission, seed_tenant, seed_webhook_delivery,
    service, user_info,
};

#[tokio::test]
#[ignore]
async fn list_store_members_refuses_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    // None of the seeded default roles grant `canviewstoreusers`, including
    // Owner, so a caller needs a role built for it before a positive control
    // against A's own store means anything.
    grant_store_permission(&pg, &a, "ethpay.store.canviewstoreusers").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::list_store_members(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to list B's store's members"
    );

    // Positive control: without this, an endpoint that refuses every caller
    // would pass the assertion above for the wrong reason.
    let _ = server::api::stores::list_store_members(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to list A's own store's members");
}

#[tokio::test]
#[ignore]
async fn get_store_webhook_refuses_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    // Both tenants need a webhook actually configured, or the positive
    // control would fail with NOT_FOUND for a reason unrelated to tenancy.
    let _a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::get_store_webhook(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to read B's store's webhook configuration"
    );

    // Positive control, same reasoning as above.
    let own = server::api::stores::get_store_webhook(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to read A's own store's webhook configuration");
    assert_eq!(own.store_id, a.store.id.0);
}

#[tokio::test]
#[ignore]
async fn get_token_policy_refuses_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::get_token_policy(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to read B's store's token policy"
    );

    // Positive control: without this, an endpoint that refuses every caller
    // would pass the assertion above for the wrong reason. No policy is
    // configured, so success here is `Ok(None)`, not an error.
    let _ = server::api::stores::get_token_policy(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to read A's own store's token policy");
}

/// All three handlers above take the same `AuthenticatedUser` extractor as
/// every other endpoint in this suite, so an API key reaches store settings
/// the same way a session does - member lists, webhook config and token
/// policy are exactly the kind of store-scoped data an under-scoped key
/// should not be able to widen its reach into.
#[tokio::test]
#[ignore]
async fn store_settings_via_api_key_cannot_reach_another_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let _a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    // `grant_store_permission` below replaces A's role on A's own store, so
    // the webhook and token-policy checks - which rely on the default role
    // `seed_tenant` grants - run first, before that swap happens.
    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result =
        server::api::stores::get_store_webhook(a_via_key, State(state.clone()), Path(b.store.id.0))
            .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "an API key must not read another tenant's store's webhook configuration"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own =
        server::api::stores::get_store_webhook(a_via_key, State(state.clone()), Path(a.store.id.0))
            .await
            .expect(
                "an API key must be able to read its owner's own store's webhook configuration",
            );
    assert_eq!(own.store_id, a.store.id.0);

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result =
        server::api::stores::get_token_policy(a_via_key, State(state.clone()), Path(b.store.id.0))
            .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "an API key must not read another tenant's store's token policy"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let _ =
        server::api::stores::get_token_policy(a_via_key, State(state.clone()), Path(a.store.id.0))
            .await
            .expect("an API key must be able to read its owner's own store's token policy");

    // Last, since this replaces A's role on A's own store: none of the
    // seeded default roles grant `canviewstoreusers`, so a positive control
    // against A's own store needs a role built for it, same as the
    // session-based test above.
    grant_store_permission(&state.data_service, &a, "ethpay.store.canviewstoreusers").await;

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::stores::list_store_members(
        a_via_key,
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "an API key must not list another tenant's store's members"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let _ = server::api::stores::list_store_members(a_via_key, State(state), Path(a.store.id.0))
        .await
        .expect("an API key must be able to list its owner's own store's members");
}
