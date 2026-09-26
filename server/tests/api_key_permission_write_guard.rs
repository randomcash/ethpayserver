#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `update_api_key_permissions` carries three
//! authorization guards - the ownership check, the "validate against the
//! key's actual owner, not the caller" launder guard, and the "a narrowed
//! key cannot use this endpoint to widen itself back to inherit" guard -
//! and none of them had a test that went through the real handler with a
//! real seeded owner/caller pair. `permission_scope_tests` and
//! `update_permissions_guard_tests` in `server/src/api/users.rs` only ever
//! call the three free functions in isolation with roles the test already
//! chose; a regression that swapped `owner_role` for `user.role`, or wired
//! the ownership check backwards, would compile and pass every one of them.
//! These tests drive `update_api_key_permissions` itself against a real
//! seeded database, one guard per test.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    ApiKey, ApiKeyId, Result as AuthResult, Role, Session, SessionId, SessionService, UserId,
    UserInfo,
};
use data_service::PgDataService;
use rates::NoOpRateProvider;
use server::api::AuthenticatedUser;
use server::api::users::{UpdateApiKeyPermissionsPayload, update_api_key_permissions};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;

struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by this handler")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by this handler")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by this handler")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by this handler")
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

async fn seed_user(pool: &PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier, role) \
         VALUES ($1, \
         '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"AAAAAAAAAAAAAAAAAAAAAA==\"}'::jsonb, \
         '{\"ciphertext\":\"AAAA\",\"iv\":\"AAAA\",\"mac\":\"AAAA\"}'::jsonb, \
         'h', 'passkey:' || $1::text, $2)",
    )
    .bind(id)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

fn user_info(id: Uuid, role: Role) -> UserInfo {
    UserInfo {
        id: UserId(id),
        email: None,
        primary_wallet_address: None,
        created_at: Utc::now(),
        last_login_at: None,
        role,
    }
}

fn app_state(data_service: Arc<PgDataService>) -> PgAppState<UnusedSessionService> {
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

async fn seed_key(pg: &PgDataService, owner: Uuid, permissions: Option<&[String]>) -> Uuid {
    let id = ApiKeyId::new();
    let key = ApiKey {
        id,
        user_id: UserId(owner),
        name: "write-guard test key".to_string(),
        key_hash: format!("hash-{}", Uuid::new_v4()),
        key_prefix: "ak_test****".to_string(),
        is_active: true,
        created_at: Utc::now(),
        last_used_at: None,
        expires_at: None,
    };
    pg.create_api_key_with_permissions(&key, permissions)
        .await
        .expect("seed api key");
    id.0
}

/// Guard 1: a caller who is neither the key's owner nor a `ServerAdmin`
/// cannot manage the key at all - not even to inspect it via a rejection
/// that would leak whether it exists.
#[tokio::test]
#[ignore]
async fn a_non_owner_non_admin_caller_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool(), "user").await;
    let stranger = seed_user(pg.pool(), "user").await;
    let key_id = seed_key(&pg, owner, Some(&[])).await;

    let state = app_state(Arc::new(pg));

    let result = update_api_key_permissions(
        AuthenticatedUser(user_info(stranger, Role::User)),
        State(state),
        Path(key_id),
        Json(UpdateApiKeyPermissionsPayload {
            permissions: Some(vec![]),
        }),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::NOT_FOUND),
        "a caller who neither owns the key nor is a ServerAdmin must be refused"
    );
}

/// Guard 2: a `ServerAdmin` editing someone else's key cannot launder a
/// wider grant through their own role. The key's owner is a plain `User`;
/// requesting `unrestricted` must be validated against the owner's role,
/// not the admin caller's, and refused.
#[tokio::test]
#[ignore]
async fn an_admin_cannot_launder_unrestricted_onto_a_non_admins_key() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool(), "user").await;
    let admin = seed_user(pg.pool(), "server_admin").await;
    let key_id = seed_key(&pg, owner, Some(&[])).await;

    let state = app_state(Arc::new(pg));

    let result = update_api_key_permissions(
        AuthenticatedUser(user_info(admin, Role::ServerAdmin)),
        State(state),
        Path(key_id),
        Json(UpdateApiKeyPermissionsPayload {
            permissions: Some(vec![auth::Permission::Unrestricted.as_policy().to_string()]),
        }),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::BAD_REQUEST),
        "granting unrestricted on a non-admin's key must be validated against the owner's role, not the caller's"
    );
}

/// Success path: an owner narrowing their own key to a real store
/// permission gets back a 200 with the new scope reflected in the response
/// body - the three guard tests above only ever prove a rejection path,
/// which cannot distinguish "the handler composes correctly" from "the
/// happy path is broken too".
#[tokio::test]
#[ignore]
async fn an_owner_narrowing_their_own_key_succeeds_and_echoes_the_new_scope() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool(), "user").await;
    let key_id = seed_key(&pg, owner, None).await;

    let state = app_state(Arc::new(pg));
    let requested = vec![auth::Permission::StoreCreateInvoice.as_policy().to_string()];

    let result = update_api_key_permissions(
        AuthenticatedUser(user_info(owner, Role::User)),
        State(state),
        Path(key_id),
        Json(UpdateApiKeyPermissionsPayload {
            permissions: Some(requested.clone()),
        }),
    )
    .await;

    let response = result.expect("a legitimate self-narrowing request must succeed");
    assert_eq!(
        response.0.permissions.as_deref(),
        Some(requested.as_slice()),
        "the response must echo the scope that was actually persisted"
    );
}

/// Guard 3: a key that has been narrowed away from `unrestricted`
/// authenticates as a plain `User` (see `validate_api_key`'s downgrade).
/// This handler must not let that same narrowed request clear its own
/// key's permissions back to `None` ("inherit the owner's role in full") -
/// that would let a narrowed key hand itself back the access it was
/// narrowed away from.
#[tokio::test]
#[ignore]
async fn a_narrowed_caller_cannot_clear_its_own_key_back_to_inherit() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool(), "server_admin").await;
    let key_id = seed_key(&pg, owner, Some(&[])).await;

    let state = app_state(Arc::new(pg));

    // The caller authenticates as `Role::User`, exactly as `validate_api_key`
    // would resolve it for this same narrowed key - not the owner's stored
    // `server_admin` role.
    let result = update_api_key_permissions(
        AuthenticatedUser(user_info(owner, Role::User)),
        State(state),
        Path(key_id),
        Json(UpdateApiKeyPermissionsPayload { permissions: None }),
    )
    .await;

    assert_eq!(
        result.err(),
        Some(StatusCode::BAD_REQUEST),
        "a narrowed key must not be able to clear itself back to inheriting full access"
    );
}
