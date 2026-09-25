//! Dashboard: aggregate counters and volume, scoped by `get_stores_for_user`
//! rather than a client-supplied `store_id` - the leak to guard against here
//! is another tenant's rows folding into the caller's own totals.

use std::sync::Arc;

use axum::extract::{Query, State};

use server::api::AuthenticatedUser;

use crate::support::{app_state, seed_tenant, service, user_info};

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
