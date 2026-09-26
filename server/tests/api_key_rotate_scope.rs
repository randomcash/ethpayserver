#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `rotate_api_key_atomic` carries the old key's
//! permission scope onto its replacement - "rotation swaps the secret, it
//! does not widen what the key can do" - but nothing asserted the rotated
//! key actually keeps that scope. A regression that dropped the
//! `permissions` argument, or passed the wrong key's scope, would compile
//! and pass every existing test: none of them rotate a key at all. This
//! drives the real `rotate_api_key` handler against a real seeded database
//! and checks the replacement's scope both in the handler's own response
//! and in what was actually persisted.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
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
use server::api::users::rotate_api_key;
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

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier, role) \
         VALUES ($1, \
         '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"AAAAAAAAAAAAAAAAAAAAAA==\"}'::jsonb, \
         '{\"ciphertext\":\"AAAA\",\"iv\":\"AAAA\",\"mac\":\"AAAA\"}'::jsonb, \
         'h', 'passkey:' || $1::text, 'user')",
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
        created_at: Utc::now(),
        last_login_at: None,
        role: Role::User,
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
        name: "rotate test key".to_string(),
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

/// A key narrowed to a single store permission keeps exactly that scope
/// after rotation - both in the handler's response and in what was actually
/// written for the replacement key - rather than rotation resetting it back
/// to full/`unrestricted` access.
#[tokio::test]
#[ignore]
async fn rotating_a_scoped_key_carries_its_scope_to_the_replacement() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let scope = vec![auth::Permission::StoreCreateInvoice.as_policy().to_string()];
    let key_id = seed_key(&pg, owner, Some(&scope)).await;

    let pg = Arc::new(pg);
    let state = app_state(pg.clone());

    let (_, response) = rotate_api_key(
        AuthenticatedUser(user_info(owner)),
        State(state),
        Path(key_id),
    )
    .await
    .expect("rotating an active, non-deprecated key must succeed");

    assert_eq!(
        response.permissions.as_deref(),
        Some(scope.as_slice()),
        "the rotation response must carry over the old key's scope"
    );

    let persisted = pg
        .get_api_key_auth_info_by_id(response.id)
        .await
        .expect("look up the replacement key")
        .expect("replacement key must exist");
    assert_eq!(
        persisted.permissions.as_deref(),
        Some(scope.as_slice()),
        "the replacement key must be persisted with the same scope, not full/unrestricted access"
    );
}
