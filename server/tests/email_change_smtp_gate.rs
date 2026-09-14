#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Proves the SMTP-unconfigured guard in `request_email_change`
//! (server/src/api/users.rs) fires before any pending state is created.
//!
//! `EmailSender::is_configured` is already unit-tested in isolation
//! (server/src/services/email.rs), but that only pins the sender's own
//! answer, not that the handler actually checks it before writing a row.
//! This calls the handler itself, with a `NoopEmailSender` wired into a real
//! `PgAppState`, so a regression that reorders the check after
//! `create_email_change_request` would turn this red.
//!
//! Needs a real Postgres and is `#[ignore]`d, matching the convention
//! `data-service`'s own DB-backed tests use (see
//! `data-service/src/postgres/integration_tests`): set `DATABASE_URL` and
//! run with `--ignored`. Skips (rather than failing) when it is unset, same
//! as those tests, so the default `cargo test` run stays DB-free.

use std::sync::Arc;

use async_trait::async_trait;
use auth::{Role, Session, SessionId, SessionService, UserId, UserInfo};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use data_service::PgDataService;
use rates::NoOpRateProvider;
use server::api::FreshlyAuthenticatedUser;
use server::api::users::{RequestEmailChangePayload, request_email_change};
use server::services::email::NoopEmailSender;
use server::state::{AppState, PgAppState};
use sqlx::PgPool;
use uuid::Uuid;

/// Never actually called: `request_email_change` only touches
/// `state.data_service` and `state.email_sender`, and this test constructs
/// `FreshlyAuthenticatedUser` directly rather than going through the
/// extractor, so nothing here reaches the auth service.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> auth::Result<(UserInfo, Session)> {
        unimplemented!("not exercised by request_email_change")
    }

    async fn logout(&self, _session_id: SessionId) -> auth::Result<()> {
        unimplemented!("not exercised by request_email_change")
    }

    async fn logout_all(&self, _session_id: SessionId) -> auth::Result<()> {
        unimplemented!("not exercised by request_email_change")
    }

    async fn cleanup_stale_sessions(&self) -> auth::Result<u64> {
        unimplemented!("not exercised by request_email_change")
    }
}

async fn state() -> Option<PgAppState<UnusedSessionService>> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(AppState::new(
        Arc::new(PgDataService::new(pool)),
        Arc::new(UnusedSessionService),
        None,
        Arc::new(NoOpRateProvider),
        Arc::new(NoopEmailSender),
    ))
}

async fn seed_user(pool: &PgPool, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, $2, \
         '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"AAAAAAAAAAAAAAAAAAAAAA==\"}'::jsonb, \
         '{\"ciphertext\":\"AAAA\",\"iv\":\"AAAA\",\"mac\":\"AAAA\"}'::jsonb, \
         'original-hash', 'email:' || $2)",
    )
    .bind(id)
    .bind(email)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user)
        .execute(pool)
        .await;
}

#[tokio::test]
#[ignore]
async fn smtp_unconfigured_rejects_before_creating_a_pending_request() {
    let Some(state) = state().await else {
        return;
    };
    let pool = state.data_service.pool().clone();
    let user_id = seed_user(&pool, "existing@example.com").await;

    let user = UserInfo {
        id: UserId(user_id),
        email: Some("existing@example.com".to_string()),
        primary_wallet_address: None,
        created_at: Utc::now(),
        last_login_at: None,
        role: Role::User,
    };

    let result = request_email_change(
        FreshlyAuthenticatedUser(user),
        State(state),
        Json(RequestEmailChangePayload {
            new_email: "new@example.com".to_string(),
        }),
    )
    .await;

    match result {
        Err((status, _)) => assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE),
        Ok(_) => panic!("must fail loudly when SMTP is unconfigured, not accept the change"),
    }

    let pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM email_change_requests WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("count pending requests");
    assert_eq!(
        pending, 0,
        "the SMTP-unconfigured path must not leave a pending row behind"
    );

    cleanup(&pool, user_id).await;
}
