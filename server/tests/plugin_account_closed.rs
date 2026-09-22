#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `services::plugins::account_closed`'s own tests exercise `notify_account_closed`
//! and the dispatch adapter in isolation - neither proves the real
//! `DELETE /users/me` handler ever calls it. This repo has shipped that exact
//! gap before (a capability that works, wired to nothing) more than once, so
//! this is the same shape of test `plugin_invoice_creation_filter.rs` and
//! `event_consumer::tests::own_store_payments` exist to be: drive the real
//! handler with a recording observer registered on `AppState` and check it
//! was actually called.
//!
//! Calls `delete_account` directly rather than through the router, the same
//! way `plugin_invoice_creation_filter.rs` calls `create_invoice`: the
//! extractors it takes are plain data, so nothing here depends on routing.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::{Query, State};
use sqlx::PgPool;
use uuid::Uuid;

use auth::{Result as AuthResult, Role, Session, SessionId, SessionService, UserId, UserInfo};
use data_service::PgDataService;
use rates::NoOpRateProvider;
use server::api::AuthenticatedUser;
use server::api::users::{DeleteAccountQuery, delete_account};
use server::services::RedisEVMMonitor;
use server::services::plugins::AccountClosedObserver;
use server::state::PgAppState;

struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by delete_account")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by delete_account")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by delete_account")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by delete_account")
    }
}

#[derive(Default)]
struct RecordingObserver {
    seen: Mutex<Vec<UserId>>,
}

#[async_trait]
impl AccountClosedObserver for RecordingObserver {
    async fn account_closed(&self, account_id: UserId) {
        self.seen.lock().unwrap().push(account_id);
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

/// A bare passkey-only account: no email, no store, nothing an
/// `account_deletion_blockers` check would refuse.
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

fn app_state(
    data_service: Arc<PgDataService>,
    observers: Vec<Arc<dyn AccountClosedObserver>>,
) -> PgAppState<UnusedSessionService> {
    let mut state = PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    );
    state.account_closed_observers = observers;
    state
}

/// The gap this whole file exists to close: a registered plugin actually
/// hears about a real, self-service `DELETE /users/me`.
#[tokio::test]
#[ignore]
async fn a_deleted_account_notifies_every_registered_plugin() {
    let Some(pg) = service().await else {
        return;
    };
    let id = seed_user(pg.pool()).await;

    let observer = Arc::new(RecordingObserver::default());
    let observers: Vec<Arc<dyn AccountClosedObserver>> = vec![observer.clone()];
    let state = app_state(Arc::new(pg), observers);

    let result = delete_account(
        AuthenticatedUser(user_info(id)),
        State(state),
        Query(DeleteAccountQuery {
            confirm: id.to_string(),
        }),
    )
    .await;

    assert!(result.is_ok(), "a bare account must delete cleanly");
    assert_eq!(
        observer.seen.lock().unwrap().as_slice(),
        [UserId(id)],
        "the plugin must be told which account closed"
    );
}

/// The negative case: a refused deletion (wrong confirmation) must not tell a
/// plugin the account is gone when it very much still exists.
#[tokio::test]
#[ignore]
async fn a_refused_deletion_notifies_nobody() {
    let Some(pg) = service().await else {
        return;
    };
    let id = seed_user(pg.pool()).await;

    let observer = Arc::new(RecordingObserver::default());
    let observers: Vec<Arc<dyn AccountClosedObserver>> = vec![observer.clone()];
    let state = app_state(Arc::new(pg), observers);

    let result = delete_account(
        AuthenticatedUser(user_info(id)),
        State(state),
        Query(DeleteAccountQuery {
            confirm: "not the right confirmation".to_string(),
        }),
    )
    .await;

    assert!(result.is_err(), "a wrong confirmation must refuse deletion");
    assert!(
        observer.seen.lock().unwrap().is_empty(),
        "an account that was never deleted must never be reported closed"
    );
}
