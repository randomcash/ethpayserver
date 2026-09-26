//! Proves `StoreScopedUser` actually authenticates an `Authorization: Bearer
//! ak_...` header, through the real `Router` -> `FromRequestParts` pipeline a
//! caller reaches it by - not a hand-built `StoreScopedUser(user_info, scope)`
//! the way every other call site in this crate exercises it (see
//! `stores::tests::admin_store_scoped_user`). A pure-function test of
//! `key_grants_store_permission` alone cannot catch a broken wire-up between
//! this extractor and `validate_api_key`; only a real request through a real
//! router can.
//!
//! Needs Postgres (`DATABASE_URL`) because `validate_api_key` queries the
//! `api_keys` and `users` tables - see `handler_test_service`-style gating
//! elsewhere in this crate (e.g. `stores::tests`).

use super::*;
use auth::Policies;
use axum::{Router, body::Body, extract::State, http::Request, routing::get};
use data_service::PgDataService;
use tower::ServiceExt;
use uuid::Uuid;

/// Exists only to give `PgAppState<A>` a concrete auth-service type -
/// these tests only ever exercise the API-key branch, never the session
/// one.
struct NoSessionService;

#[async_trait::async_trait]
impl SessionService for NoSessionService {
    async fn validate_session(
        &self,
        _session_id: SessionId,
    ) -> auth::Result<(UserInfo, auth::Session)> {
        Err(auth::AuthError::InvalidCredentials)
    }

    async fn logout(&self, _session_id: SessionId) -> auth::Result<()> {
        Err(auth::AuthError::InvalidCredentials)
    }

    async fn logout_all(&self, _session_id: SessionId) -> auth::Result<()> {
        Err(auth::AuthError::InvalidCredentials)
    }

    async fn cleanup_stale_sessions(&self) -> auth::Result<u64> {
        Err(auth::AuthError::InvalidCredentials)
    }
}

async fn test_service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    PgDataService::connect(&database_url).await.ok()
}

fn test_state(service: PgDataService) -> PgAppState<NoSessionService> {
    crate::state::AppState::new(
        std::sync::Arc::new(service),
        std::sync::Arc::new(NoSessionService),
        None,
        std::sync::Arc::new(rates::NoOpRateProvider),
        std::sync::Arc::new(crate::services::email::NoopEmailSender),
    )
}

async fn seed_user(pool: &sqlx::PgPool) -> Uuid {
    let user_id = Uuid::new_v4();
    sqlx::query(
            "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
             recovery_verification_hash, kdf_salt_identifier, role) \
             VALUES ($1, \
             '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"AAAAAAAAAAAAAAAAAAAAAA==\"}'::jsonb, \
             '{\"ciphertext\":\"AAAA\",\"iv\":\"AAAA\",\"mac\":\"AAAA\"}'::jsonb, \
             'h', 'passkey:' || $1::text, 'user')",
        )
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed user");
    user_id
}

async fn seed_api_key(pool: &sqlx::PgPool, user_id: Uuid, raw_key: &str, permissions: &[String]) {
    sqlx::query(
            "INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix, is_active, created_at, permissions) \
             VALUES ($1, $2, 'reachability test key', $3, 'ak_test****', true, NOW(), $4)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(hash_api_key(raw_key))
        .bind(permissions)
        .execute(pool)
        .await
        .expect("seed api key");
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
struct EchoedScope {
    user_id: Uuid,
    scope: Option<Vec<String>>,
}

async fn echo_store_scope<A>(
    StoreScopedUser(user, scope): StoreScopedUser,
    State(_): State<PgAppState<A>>,
) -> axum::Json<EchoedScope>
where
    A: SessionService + 'static,
{
    axum::Json(EchoedScope {
        user_id: user.id.0,
        scope,
    })
}

fn echo_app(state: PgAppState<NoSessionService>) -> Router {
    Router::new()
        .route("/echo", get(echo_store_scope::<NoSessionService>))
        .with_state(state)
}

#[tokio::test]
#[ignore]
async fn an_api_key_authenticates_through_store_scoped_user_with_its_stored_scope() {
    let Some(service) = test_service().await else {
        return;
    };
    let pool = service.pool().clone();
    let user_id = seed_user(&pool).await;
    let raw_key = format!("ak_reach_{}", Uuid::new_v4());
    seed_api_key(
        &pool,
        user_id,
        &raw_key,
        &[Policies::STORE_CREATE_INVOICE.to_string()],
    )
    .await;

    let req = Request::builder()
        .method("GET")
        .uri("/echo")
        .header("Authorization", format!("Bearer {raw_key}"))
        .body(Body::empty())
        .unwrap();

    let resp = echo_app(test_state(service)).oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a real API key must authenticate through the real StoreScopedUser extractor"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let echoed: EchoedScope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        echoed,
        EchoedScope {
            user_id,
            scope: Some(vec![Policies::STORE_CREATE_INVOICE.to_string()]),
        },
        "the extractor must carry the key's actual stored scope, not drop it"
    );
}

#[tokio::test]
#[ignore]
async fn an_invalid_api_key_is_still_rejected_by_store_scoped_user() {
    let Some(service) = test_service().await else {
        return;
    };

    let req = Request::builder()
        .method("GET")
        .uri("/echo")
        .header("Authorization", "Bearer ak_does_not_exist_0000000000")
        .body(Body::empty())
        .unwrap();

    let resp = echo_app(test_state(service)).oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "StoreScopedUser must not accept a key that was never issued"
    );
}
