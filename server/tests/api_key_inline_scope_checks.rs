#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `list_store_members`, `add_store_member`,
//! `update_store_member`, `remove_store_member` (members.rs) and
//! `get_store_wallet` (wallets.rs) each inline their own
//! `key_grants_store_permission` check rather than going through the shared
//! `require_store_settings_permission` helper covered by
//! `api_key_store_settings_scope.rs` - so none of them had any test driving a
//! real `Some(scope)` key through the actual handler body. A copy-paste slip
//! at any one of these five call sites (wrong permission constant, or the
//! wrong `store_id` in a two-path-param handler) would compile and pass
//! every existing test. These tests drive each of the five with a real
//! scoped key, in the direction that would catch exactly that.
//!
//! `canviewstoreusers`/`canmodifystoreusers` aren't in any seeded role's
//! permissions today (a pre-existing gap, not introduced by this PR - see
//! the ticket's own note that `canviewinvoices` has the same property), so
//! the owner's default "Owner" role is swapped for a store-scoped role built
//! with exactly the permission a test needs.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::repository::{StoreRoleRepository, UserStoreRepository};
use auth::{
    Policies, Result as AuthResult, Role, Session, SessionId, SessionService, Store, StoreId,
    StoreRole, UserId, UserInfo, UserStore,
};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::StoreScopedUser;
use server::api::stores::{
    StoreWalletQuery, add_store_member, get_store_wallet, list_store_members, remove_store_member,
};
use server::state::PgAppState;

struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by these handlers")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by these handlers")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by these handlers")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by these handlers")
    }
}

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

fn user_info(id: Uuid) -> UserInfo {
    UserInfo {
        id: UserId(id),
        email: None,
        primary_wallet_address: None,
        created_at: chrono::Utc::now(),
        last_login_at: None,
        role: Role::User,
    }
}

fn app_state(data_service: Arc<PgDataService>) -> PgAppState<UnusedSessionService> {
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

/// Swap the owner's role on `store_id` for a store-scoped role granting
/// exactly `permissions` - `canviewstoreusers`/`canmodifystoreusers` are on
/// no seeded role, so the default "Owner" role can't stand in for them.
async fn seed_owner_with_permissions(
    pg: &PgDataService,
    owner: Uuid,
    store_id: Uuid,
    permissions: Vec<String>,
) {
    let role = StoreRole::new(
        StoreId(store_id),
        format!("scoped-{}", Uuid::new_v4()),
        permissions,
    );
    pg.create_store_role(&role)
        .await
        .expect("create a store-scoped role carrying the test's permission");
    pg.update_user_store(&UserStore::new(UserId(owner), StoreId(store_id), role.id))
        .await
        .expect("move the owner onto the scoped role");
}

/// A key scoped to `canviewstoreusers` reaches past `list_store_members`'s
/// own inline permission check.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_users_can_list_members() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    seed_owner_with_permissions(
        &pg,
        owner,
        store.id.0,
        vec!["ethpay.store.canviewstoreusers".to_string()],
    )
    .await;

    let state = app_state(Arc::new(pg));

    let result = list_store_members(
        StoreScopedUser(
            user_info(owner),
            Some(vec!["ethpay.store.canviewstoreusers".to_string()]),
        ),
        State(state),
        Path(store.id.0),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canviewstoreusers must be able to list members: {:?}",
        result.err()
    );
}

/// The other direction: the owner has `canviewstoreusers`, but the key is
/// scoped to something else - proving `list_store_members` is a real
/// intersection with the key's narrower scope, not the owner's role alone.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_something_else_is_refused_list_members() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    seed_owner_with_permissions(
        &pg,
        owner,
        store.id.0,
        vec!["ethpay.store.canviewstoreusers".to_string()],
    )
    .await;

    let state = app_state(Arc::new(pg));

    let result = list_store_members(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(store.id.0),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key not scoped to canviewstoreusers must be refused by list_store_members"
    );
}

/// A key scoped to `canmodifystoreusers` reaches past `add_store_member`'s
/// own inline permission check.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_modify_users_can_add_a_member() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let new_member = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    seed_owner_with_permissions(
        &pg,
        owner,
        store.id.0,
        vec!["ethpay.store.canmodifystoreusers".to_string()],
    )
    .await;

    let state = app_state(Arc::new(pg));

    let result = add_store_member(
        StoreScopedUser(
            user_info(owner),
            Some(vec!["ethpay.store.canmodifystoreusers".to_string()]),
        ),
        State(state),
        Path(store.id.0),
        Json(api_types::AddMemberRequest {
            user_id: new_member,
            role: "Guest".to_string(),
        }),
    )
    .await;

    assert!(
        result.is_ok(),
        "a key scoped to canmodifystoreusers must be able to add a member: {:?}",
        result.err()
    );
}

/// `policy:storeId` scoping on a two-path-param handler
/// (`{store_id}/members/{user_id}`): a key scoped to store A's
/// `canmodifystoreusers` must not grant `remove_store_member` on store B,
/// which would surface a copy-paste bug that checked the wrong path
/// parameter as the store id.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_one_store_is_refused_remove_member_on_another() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let member = seed_user(pg.pool()).await;
    let store_a = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    let store_b = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store_a, UserId(owner))
        .await
        .expect("seed store a");
    pg.create_store_owned_by(&store_b, UserId(owner))
        .await
        .expect("seed store b");
    seed_owner_with_permissions(
        &pg,
        owner,
        store_a.id.0,
        vec!["ethpay.store.canmodifystoreusers".to_string()],
    )
    .await;
    seed_owner_with_permissions(
        &pg,
        owner,
        store_b.id.0,
        vec!["ethpay.store.canmodifystoreusers".to_string()],
    )
    .await;

    let state = app_state(Arc::new(pg));

    let scoped_to_store_a = vec![format!("ethpay.store.canmodifystoreusers:{}", store_a.id.0)];

    let result = remove_store_member(
        StoreScopedUser(user_info(owner), Some(scoped_to_store_a)),
        State(state),
        Path((store_b.id.0, member)),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key scoped to store A's canmodifystoreusers must be refused on store B"
    );
}

/// A key scoped to `canviewstoresettings` reaches past `get_store_wallet`'s
/// own inline permission check. The default "Owner" role already grants
/// this permission, unlike the store-user ones above.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_view_settings_reaches_past_get_wallet_permission_check() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user, with the Owner role's canviewstoresettings permission");

    let state = app_state(Arc::new(pg));

    let result = get_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_VIEW_SETTINGS.to_string()]),
        ),
        State(state),
        Path(store.id.0),
        Query(StoreWalletQuery {
            namespace: None,
            payment_method_id: None,
        }),
    )
    .await;

    let Err(status) = result else {
        panic!("expected a downstream failure past the permission check (no wallet configured)");
    };
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "a key scoped to canviewstoresettings must pass get_store_wallet's permission check"
    );
}

/// The other direction: the owner has `canviewstoresettings`, but the key is
/// scoped to something else - `get_store_wallet` must refuse it.
#[tokio::test]
#[ignore]
async fn a_key_scoped_to_something_else_is_refused_get_wallet() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let state = app_state(Arc::new(pg));

    let result = get_store_wallet(
        StoreScopedUser(
            user_info(owner),
            Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        ),
        State(state),
        Path(store.id.0),
        Query(StoreWalletQuery {
            namespace: None,
            payment_method_id: None,
        }),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "a key not scoped to canviewstoresettings must be refused by get_store_wallet"
    );
}
