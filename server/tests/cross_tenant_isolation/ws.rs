//! The `/ws` feed's per-frame visibility policy, against a real database.

use server::api::ws::invoice_visible_to;

use crate::support::{seed_tenant, service, user_info, user_info_with_role};

#[tokio::test]
#[ignore]
async fn ws_feed_shows_only_the_callers_own_invoices() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let admin = crate::support::seed_user(pg.pool()).await;

    // Positive control: a member sees their own store's invoice.
    assert!(invoice_visible_to(&pg, &user_info(a.user_id), a.invoice.id.to_string()).await);
    // A foreign tenant's invoice is dropped.
    assert!(!invoice_visible_to(&pg, &user_info(a.user_id), b.invoice.id.to_string()).await);
    assert!(!invoice_visible_to(&pg, &user_info(b.user_id), a.invoice.id.to_string()).await);
    // An unresolvable invoice is dropped, never forwarded.
    assert!(!invoice_visible_to(&pg, &user_info(a.user_id), "inv_does_not_exist".into()).await);
    // A server admin sees every store.
    let admin_info = user_info_with_role(admin, auth::Role::ServerAdmin);
    assert!(invoice_visible_to(&pg, &admin_info, a.invoice.id.to_string()).await);
    assert!(invoice_visible_to(&pg, &admin_info, b.invoice.id.to_string()).await);
}
