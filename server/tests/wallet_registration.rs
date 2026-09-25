#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Proves the onboarding path the docs describe actually works: an xpub
//! `derive-xpub` prints is accepted by the real `POST /wallets` handler
//! (`create_wallet`, in `server/src/api/stores/wallets.rs`) and the addresses
//! it hands back match what the same key derives directly.
//!
//! `evm/src/wallet.rs` has a unit test pinning the encoding fix that made this
//! xpub `xpub…`-prefixed instead of `zpub……`, and `wallets.rs` has one pinning
//! that `derive_entries` agrees with `HdWallet` given a wallet row - but
//! neither goes through `validate_xpub`, `WalletWriter::create_wallet`, or a
//! real database row, which is where a version-byte regression or a
//! validation gap would actually surface. This does: it calls the handler
//! function itself, against a real Postgres, the same way
//! `email_change_smtp_gate.rs` proves its own handler's guard fires.
//!
//! Needs a real Postgres and is `#[ignore]`d, matching the convention
//! `data-service`'s own DB-backed tests use: set `DATABASE_URL` and run with
//! `--ignored`, so the default `cargo test` run stays DB-free. Unlike some
//! of those tests, this one fails loud (`expect`, not a silent early return)
//! when `DATABASE_URL` is unset - this is the test that proves the ticket's
//! mandated "register the resulting key against a real instance" step, so an
//! unset variable earning a bare pass with zero assertions run is exactly
//! the failure mode it exists to catch. This is not a claim taken on faith:
//! `.github/workflows/ci.yml`'s "Integration tests" step (currently around
//! line 294) runs `cargo nextest run -p data-service -p server --run-ignored
//! only` with `DATABASE_URL` exported two lines above it, which is `-p
//! server`, i.e. this crate, i.e. this file - and running this exact test
//! locally against a live Postgres (outside CI, by hand) passes. A
//! contributor who runs `--ignored` locally without `DATABASE_URL` set gets
//! a clear panic instead of a silent no-op.

use std::sync::Arc;

use async_trait::async_trait;
use auth::{Role, Session, SessionId, SessionService, UserId, UserInfo};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use data_service::PgDataService;
use evm::{ChainFamily, HdWallet, generate_mnemonic};
use rates::NoOpRateProvider;
use server::api::AuthenticatedUser;
use server::api::stores::{VERIFICATION_ADDRESS_COUNT, create_wallet};
use server::services::email::NoopEmailSender;
use server::state::{AppState, PgAppState};
use sqlx::PgPool;
use types::NAMESPACE_EIP155;
use uuid::Uuid;

use api_types::CreateWalletRequest;

/// Never actually called: `create_wallet` only touches `state.data_service`,
/// and this test constructs `AuthenticatedUser` directly rather than going
/// through the extractor, so nothing here reaches the auth service.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> auth::Result<(UserInfo, Session)> {
        unimplemented!("not exercised by create_wallet")
    }

    async fn logout(&self, _session_id: SessionId) -> auth::Result<()> {
        unimplemented!("not exercised by create_wallet")
    }

    async fn logout_all(&self, _session_id: SessionId) -> auth::Result<()> {
        unimplemented!("not exercised by create_wallet")
    }

    async fn cleanup_stale_sessions(&self) -> auth::Result<u64> {
        unimplemented!("not exercised by create_wallet")
    }
}

async fn state() -> Option<PgAppState<UnusedSessionService>> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");
    Some(AppState::new(
        Arc::new(PgDataService::new(pool)),
        Arc::new(UnusedSessionService),
        None,
        Arc::new(NoOpRateProvider),
        Arc::new(NoopEmailSender),
    ))
}

async fn seed_user(pool: &PgPool, id: Uuid, email: &str) {
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
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    sqlx::query("DELETE FROM wallets WHERE user_id = $1")
        .bind(user)
        .execute(pool)
        .await
        .expect("delete test wallet");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user)
        .execute(pool)
        .await
        .expect("delete test user");
}

/// The exact join the ticket asked to see walked: generate a mnemonic the way
/// `derive-xpub generate` does, print the same account xpub it would print,
/// and register it through the real HTTP handler rather than a hand-built
/// row. If `validate_xpub` rejected the encoding this repo now produces, or
/// `WalletWriter::create_wallet`/`derive_entries` disagreed with `HdWallet`
/// about what the key derives, this fails; it passed before landing this
/// test.
#[tokio::test]
#[ignore]
async fn an_xpub_derive_xpub_prints_is_accepted_by_the_real_wallet_endpoint() {
    let state = state()
        .await
        .expect("DATABASE_URL must be set to run this ignored test - CI sets it before passing --ignored");
    let pool = state.data_service.pool().clone();
    // Unique per run, not a fixed literal: a prior failed run that skipped
    // `cleanup` (reached only on the happy path) would otherwise leave a row
    // that collides on the next run's INSERT and masks a real regression
    // behind a unique-constraint error instead.
    let user_id = Uuid::new_v4();
    let email = format!("wallet-registration-{user_id}@example.com");
    seed_user(&pool, user_id, &email).await;

    let user = UserInfo {
        id: UserId(user_id),
        email: Some(email),
        primary_wallet_address: None,
        created_at: Utc::now(),
        last_login_at: None,
        role: Role::User,
    };

    let mnemonic = generate_mnemonic(24).expect("generate mnemonic");
    let wallet = HdWallet::from_mnemonic(&mnemonic, "").expect("derive wallet from mnemonic");
    let xpub = wallet
        .account_xpub_string_for(ChainFamily::Evm)
        .expect("account xpub");
    assert!(
        xpub.starts_with("xpub"),
        "derive-xpub's output must be the xpub-prefixed encoding a merchant's \
         own wallet uses, not zpub: got {xpub}"
    );

    let response = create_wallet(
        AuthenticatedUser(user),
        State(state),
        Json(CreateWalletRequest {
            xpub: xpub.clone(),
            name: None,
            namespace: NAMESPACE_EIP155.to_string(),
        }),
    )
    .await
    .expect("a derive-xpub-shaped xpub must be accepted by POST /wallets");

    let (status, Json(body)) = response;
    assert_eq!(status, StatusCode::CREATED);

    assert_eq!(
        body.verification_addresses.len(),
        VERIFICATION_ADDRESS_COUNT as usize,
        "POST /wallets returned a different number of verification addresses \
         than create_wallet is supposed to derive"
    );

    let expected: Vec<String> = (0..VERIFICATION_ADDRESS_COUNT)
        .map(|i| {
            ChainFamily::Evm.encode_address(
                wallet
                    .derive_address_for(ChainFamily::Evm, i)
                    .expect("derive address"),
            )
        })
        .collect();
    let actual: Vec<String> = body
        .verification_addresses
        .iter()
        .map(|entry| entry.address.clone())
        .collect();
    assert_eq!(
        actual, expected,
        "POST /wallets returned addresses that don't match what the same \
         mnemonic derives directly"
    );

    cleanup(&pool, user_id).await;
}
