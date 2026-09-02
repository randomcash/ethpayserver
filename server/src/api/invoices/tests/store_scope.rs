#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Authorization-boundary tests for the invoice/payment list store scope.
//!
//! Regression coverage for RCS-211. The nil UUID used to be an in-band "all
//! stores" sentinel, so `store_id=00000000-0000-0000-0000-000000000000` took
//! the `Some` arm of the scope match, skipped the admin check *and* the
//! membership check, and then dropped the `WHERE store_id` clause - handing any
//! authenticated user every invoice and payment in the deployment.
//!
//! The scope is now an `Option` with no in-band marker. These tests pin that:
//! the nil UUID must be treated as an ordinary store id, never as "all".

use super::super::verify_store_access_for_query;
use ::types::StoreId;
use async_trait::async_trait;
use auth::store::{UserStore, UserStoreInfo};
use auth::{Result, Role, UserId, UserInfo, repository::UserStoreRepository};
use axum::http::StatusCode;
use chrono::Utc;
use uuid::Uuid;

/// Membership repository stub. `member_of` is the one store the user belongs
/// to; every other store returns "not a member". Only `get_user_store` is
/// exercised by the code under test.
struct StubStores {
    member_of: Option<Uuid>,
}

#[async_trait]
impl UserStoreRepository for StubStores {
    async fn get_user_store(
        &self,
        user_id: UserId,
        store_id: StoreId,
    ) -> Result<Option<UserStore>> {
        Ok(match self.member_of {
            Some(id) if id == store_id.0 => Some(UserStore::new(
                user_id,
                store_id,
                auth::store::StoreRoleId(Uuid::new_v4()),
            )),
            _ => None,
        })
    }

    async fn add_user_to_store(&self, _: &UserStore) -> Result<()> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn get_user_stores(&self, _: UserId) -> Result<Vec<UserStore>> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn get_store_users(&self, _: StoreId) -> Result<Vec<UserStore>> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn update_user_store(&self, _: &UserStore) -> Result<()> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn remove_user_from_store(&self, _: UserId, _: StoreId) -> Result<()> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn user_has_store_permission(&self, _: UserId, _: StoreId, _: &str) -> Result<bool> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn get_user_store_info(&self, _: UserId, _: StoreId) -> Result<Option<UserStoreInfo>> {
        unimplemented!("not exercised by the store-scope gate")
    }
    async fn get_user_store_infos(&self, _: UserId) -> Result<Vec<UserStoreInfo>> {
        unimplemented!("not exercised by the store-scope gate")
    }
}

fn user_with_role(role: Role) -> UserInfo {
    UserInfo {
        id: UserId(Uuid::new_v4()),
        email: Some("merchant@example.com".to_string()),
        primary_wallet_address: None,
        created_at: Utc::now(),
        last_login_at: None,
        role,
    }
}

// =========================================================================
// RCS-211: the nil UUID must never mean "every store"
// =========================================================================

/// The exploit, pinned. A non-admin who is a member of exactly one real store
/// asks for the nil store. Before the fix this returned `Ok(None)` - no filter,
/// every tenant's rows. It must be a membership failure like any other store.
#[tokio::test]
async fn non_admin_passing_nil_store_id_is_forbidden_not_granted_all_stores() {
    let own_store = Uuid::new_v4();
    let repo = StubStores {
        member_of: Some(own_store),
    };
    let user = user_with_role(Role::User);

    let result = verify_store_access_for_query(&repo, &user, Some(Uuid::nil())).await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "nil store_id must be membership-checked, not treated as an all-stores sentinel"
    );
}

/// The other half of the old bug: even when the gate was passed, the nil id
/// suppressed the SQL filter. A member of the nil store must get it back as a
/// real, applied filter rather than `None`.
#[tokio::test]
async fn nil_store_id_is_returned_as_an_ordinary_scoped_filter() {
    let repo = StubStores {
        member_of: Some(Uuid::nil()),
    };
    let user = user_with_role(Role::User);

    let scope = verify_store_access_for_query(&repo, &user, Some(Uuid::nil()))
        .await
        .expect("member of the nil store is allowed to query it");

    assert_eq!(
        scope,
        Some(StoreId(Uuid::nil())),
        "nil must round-trip as a filter, never collapse to the unfiltered case"
    );
}

// =========================================================================
// The surrounding boundary, so the gate can't be loosened unnoticed
// =========================================================================

#[tokio::test]
async fn non_admin_without_store_id_is_rejected() {
    let repo = StubStores {
        member_of: Some(Uuid::new_v4()),
    };
    let user = user_with_role(Role::User);

    let result = verify_store_access_for_query(&repo, &user, None).await;

    assert_eq!(result.unwrap_err(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn non_admin_querying_a_store_they_do_not_belong_to_is_forbidden() {
    let repo = StubStores {
        member_of: Some(Uuid::new_v4()),
    };
    let user = user_with_role(Role::User);

    let result = verify_store_access_for_query(&repo, &user, Some(Uuid::new_v4())).await;

    assert_eq!(result.unwrap_err(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn non_admin_querying_their_own_store_is_scoped_to_it() {
    let own_store = Uuid::new_v4();
    let repo = StubStores {
        member_of: Some(own_store),
    };
    let user = user_with_role(Role::User);

    let scope = verify_store_access_for_query(&repo, &user, Some(own_store))
        .await
        .expect("member may query their own store");

    assert_eq!(scope, Some(StoreId(own_store)));
}

/// The intended all-stores path, per the RCS-171 scope decision: admins only,
/// and only by omitting `store_id` entirely.
#[tokio::test]
async fn admin_without_store_id_queries_every_store() {
    let repo = StubStores { member_of: None };
    let user = user_with_role(Role::ServerAdmin);

    let scope = verify_store_access_for_query(&repo, &user, None)
        .await
        .expect("admin may query all stores");

    assert!(scope.is_none(), "admin all-stores query must be unfiltered");
}

/// An admin naming a store they are not a member of is still scoped to that
/// store - the role widens *reach*, it must not silently widen the filter.
#[tokio::test]
async fn admin_with_store_id_stays_scoped_to_that_store() {
    let other_store = Uuid::new_v4();
    let repo = StubStores { member_of: None };
    let user = user_with_role(Role::ServerAdmin);

    let scope = verify_store_access_for_query(&repo, &user, Some(other_store))
        .await
        .expect("admin may query any store");

    assert_eq!(scope, Some(StoreId(other_store)));
}

// =========================================================================
// Handler wiring
// =========================================================================

/// The tests above pin `verify_store_access_for_query`, which was always
/// correct - the RCS-211 bug was a *second*, sentinel-based copy of the scope
/// logic inlined in the two list handlers. Unit tests of the helper therefore
/// cannot catch that regression class on their own, and there is no handler
/// harness (`PgAppState` is pinned to the concrete Postgres service, so the
/// handlers are unreachable without a live database).
///
/// This closes the gap the cheap way: the handlers must own no nil-UUID logic
/// of their own. If someone reintroduces a sentinel instead of calling the
/// helper, this fails.
#[test]
fn list_handlers_do_not_reintroduce_a_nil_uuid_sentinel() {
    for (name, src) in [
        ("list.rs", include_str!("../list.rs")),
        ("payments.rs", include_str!("../payments.rs")),
        ("csv_export.rs", include_str!("../csv_export.rs")),
    ] {
        assert!(
            !src.contains("nil()"),
            "{name} references a nil UUID. Store scope must stay an Option \
             resolved by verify_store_access_for_query - an in-band sentinel a \
             caller can also supply is what RCS-211 was."
        );
        assert!(
            src.contains("verify_store_access_for_query"),
            "{name} must resolve store scope through verify_store_access_for_query"
        );
    }
}
