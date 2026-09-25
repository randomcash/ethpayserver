//! Dashboard: aggregate counters and volume, scoped by `get_stores_for_user`
//! rather than a client-supplied `store_id` - the leak to guard against here
//! is another tenant's rows folding into the caller's own totals.

use std::sync::Arc;

use axum::extract::{Query, State};

use server::api::AuthenticatedUser;

use crate::support::{app_state, authenticate_via_bearer, seed_tenant, service, user_info};

#[tokio::test]
#[ignore]
async fn get_stats_never_counts_another_tenants_data() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let stats =
        server::api::dashboard::get_stats(AuthenticatedUser(user_info(a.user_id)), State(state))
            .await
            .expect("a merchant must be able to read their own dashboard stats");

    assert_eq!(
        stats.total_stores, 1,
        "A's store count must not include B's store"
    );
    assert_eq!(
        stats.total_invoices, 1,
        "A's invoice count must not include B's invoice"
    );
}

#[tokio::test]
#[ignore]
async fn get_analytics_never_counts_another_tenants_data() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let analytics = server::api::dashboard::get_analytics(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::dashboard::AnalyticsQuery { days: None }),
    )
    .await
    .expect("a merchant must be able to read their own dashboard analytics");

    assert_eq!(
        analytics.total_payments, 1,
        "A's payment volume must not include B's payment"
    );
}

/// Both handlers take the same `AuthenticatedUser` extractor as every other
/// endpoint in this suite, so an API key reaches the dashboard the same way a
/// session does - an API key that folded another tenant's rows into these
/// aggregates would leak exactly what the session-based tests above prove it
/// cannot.
#[tokio::test]
#[ignore]
async fn dashboard_via_api_key_never_counts_another_tenants_data() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let stats = server::api::dashboard::get_stats(a_via_key, State(state.clone()))
        .await
        .expect("an API key must be able to read its owner's own dashboard stats");
    assert_eq!(
        stats.total_stores, 1,
        "A's api-key-authenticated store count must not include B's store"
    );
    assert_eq!(
        stats.total_invoices, 1,
        "A's api-key-authenticated invoice count must not include B's invoice"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let analytics = server::api::dashboard::get_analytics(
        a_via_key,
        State(state),
        Query(server::api::dashboard::AnalyticsQuery { days: None }),
    )
    .await
    .expect("an API key must be able to read its owner's own dashboard analytics");
    assert_eq!(
        analytics.total_payments, 1,
        "A's api-key-authenticated payment volume must not include B's payment"
    );
}
