//! Wallets: list, get-by-id, and the store-wallet binding endpoints
//! (read and configure), across tenants.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use server::api::AuthenticatedUser;
use server::api::stores::{SetStoreWalletRequest, StoreWalletResult};

use crate::support::{app_state, seed_tenant, service, user_info};

#[tokio::test]
#[ignore]
async fn wallets_are_scoped_to_the_owning_account() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let listed = server::api::stores::list_wallets(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
    )
    .await
    .expect("listing one's own wallets must succeed");
    assert!(
        listed.iter().any(|w| w.id == a.wallet.id),
        "A's own wallet must be listed"
    );
    assert!(
        !listed.iter().any(|w| w.id == b.wallet.id),
        "B's wallet leaked into A's wallet list"
    );

    let result = server::api::stores::get_wallet_by_id(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.wallet.id),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch B's wallet by id"
    );

    // Positive control: without this, `get_wallet_by_id` refusing every
    // caller, including one asking about their own wallet, would pass the
    // assertion above for the wrong reason.
    let own = server::api::stores::get_wallet_by_id(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.wallet.id),
    )
    .await
    .expect("A must be able to fetch A's own wallet by id");
    assert_eq!(own.id, a.wallet.id);
}

#[tokio::test]
#[ignore]
async fn store_wallet_endpoints_refuse_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::stores::get_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
        Query(server::api::stores::StoreWalletQuery {
            namespace: None,
            // The bare form: these tests are about who may read a store's
            // wallet at all, not about which wallet a payment method
            // resolves to. Method-scoped resolution has its own tests.
            payment_method_id: None,
        }),
    )
    .await;
    assert_eq!(
        get_result.err(),
        Some(StatusCode::FORBIDDEN),
        "A must not be able to read B's store wallet"
    );

    let configure_result = server::api::stores::configure_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
        axum::Json(SetStoreWalletRequest {
            wallet_id: a.wallet.id,
        }),
    )
    .await;
    assert_eq!(
        configure_result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to pin a wallet onto B's store"
    );

    // Positive control: without this, `get_store_wallet` refusing every
    // caller, including one asking about their own store, would pass the
    // assertion above for the wrong reason.
    let own = server::api::stores::get_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
        Query(server::api::stores::StoreWalletQuery {
            namespace: None,
            // The bare form: these tests are about who may read a store's
            // wallet at all, not about which wallet a payment method
            // resolves to. Method-scoped resolution has its own tests.
            payment_method_id: None,
        }),
    )
    .await
    .expect("A must be able to read A's own store wallet");
    let StoreWalletResult::Bare(own) = own else {
        panic!("bare form (no payment_method_id) must resolve to StoreWalletResult::Bare");
    };
    assert_eq!(own.wallet.id, a.wallet.id);
}

#[tokio::test]
#[ignore]
async fn store_wallet_override_refuses_a_wallet_from_another_account() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    // A owns this store, so the permission check passes; the repository is
    // what must refuse pointing it at a wallet from B's account.
    let result = server::api::stores::configure_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(a.store.id.0),
        axum::Json(SetStoreWalletRequest {
            wallet_id: b.wallet.id,
        }),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A's own store must not be pinnable to B's wallet"
    );

    // Positive control: without this, `configure_store_wallet` refusing
    // every wallet id, including the caller's own, would pass the
    // assertion above for the wrong reason.
    let own = server::api::stores::configure_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
        axum::Json(SetStoreWalletRequest {
            wallet_id: a.wallet.id,
        }),
    )
    .await
    .expect("A must be able to pin A's own store to A's own wallet");
    assert_eq!(own.wallet.id, a.wallet.id);
}
