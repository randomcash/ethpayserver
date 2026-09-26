//! Shared fixtures for the account/store hard-delete test suite: seeding
//! rows, building an authenticated caller, and constructing `PgAppState`
//! with or without a live monitor.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{Result as AuthResult, Role, Session, SessionId, SessionService, UserId, UserInfo};
use data_service::PgDataService;
use rates::NoOpRateProvider;
use server::api::admin::E2E_STORE_OWNER_ID;
use server::api::{AdminAuth, AuthenticatedUser};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;

/// Not exercised: neither handler calls back into session management, only
/// reads the already-authenticated `AdminAuth` this test constructs directly.
pub(crate) struct UnusedSessionService;

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

pub(crate) async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    // `?` here would collapse "not configured" and "configured but
    // unreachable" into the same skip, and a skipped test reports the same
    // green result as a passing one. These four tests are the only
    // verification that the server-admin refusal, the financial-history
    // refusal and the delete cascade actually hold - a DB that is set but
    // briefly unreachable must fail loudly, not silently report success.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .unwrap_or_else(|e| panic!("DATABASE_URL is set but connecting failed: {e}"));
    Some(PgDataService::new(pool))
}

/// `plugin_invoice_creation_filter.rs`'s `seed_user` uses `'{}'::jsonb` for
/// both blobs and gets away with it because nothing there ever reads a row
/// back through `row_to_user`. This test does - `delete_user_account` calls
/// `get_user` - and `row_to_user` deserializes both into `KdfParams` and
/// `EncryptedBlob`, so an empty object fails with "missing field `algorithm`"
/// before the handler under test ever runs.
pub(crate) async fn seed_user_with_id(pool: &PgPool, id: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier, role) \
         VALUES ($1, \
           '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"\"}'::jsonb, \
           '{\"ciphertext\":\"\",\"iv\":\"\",\"mac\":\"\"}'::jsonb, \
           'h', 'passkey:' || $1::text, $2)",
    )
    .bind(id)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
}

pub(crate) async fn seed_user(pool: &PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    seed_user_with_id(pool, id, role).await;
    id
}

/// The only account `hard_delete_store` will ever act on - see
/// `E2E_STORE_OWNER_ID`. A store owned by anyone else, however it is named,
/// must be refused, so the success-path tests below need a store actually
/// owned by this exact id, not an arbitrary one.
pub(crate) async fn seed_e2e_owner(pool: &PgPool) -> Uuid {
    let id: Uuid = E2E_STORE_OWNER_ID.parse().expect("valid uuid literal");
    seed_user_with_id(pool, id, "user").await;
    id
}

pub(crate) async fn seed_invoice(pool: &PgPool, store: Uuid) -> String {
    let id = format!("inv-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO invoices (id, store_id, currency, amount, expires_at) \
         VALUES ($1, $2, 'USD', 10, now() + interval '1 hour')",
    )
    .bind(&id)
    .bind(store)
    .execute(pool)
    .await
    .expect("seed invoice");
    id
}

pub(crate) async fn seed_payment(pool: &PgPool, invoice: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payments (id, invoice_id, chain_id, asset_type, asset_symbol, amount, tx_hash) \
         VALUES ($1, $2, 'eip155:11155111', 'native', 'ETH', 1, $3)",
    )
    .bind(id)
    .bind(invoice)
    .bind(format!("0x{}", Uuid::new_v4().simple()))
    .execute(pool)
    .await
    .expect("seed payment");
    id
}

/// A refund against `store` - the other half of `ensure_no_payout_or_refund`,
/// which ORs a payout check with this one. `seed_payout` alone only exercises
/// the left side of that `||`; without this, a refund-only store never got
/// a test.
pub(crate) async fn seed_refund(pool: &PgPool, store: Uuid, invoice: &str, payment: Uuid) {
    sqlx::query(
        "INSERT INTO refunds (id, invoice_id, payment_id, store_id, to_address, chain_id, asset_symbol, amount) \
         VALUES ($1, $2, $3, $4, '0x0000000000000000000000000000000000000000', 'eip155:11155111', 'ETH', '1')",
    )
    .bind(Uuid::new_v4())
    .bind(invoice)
    .bind(payment)
    .bind(store)
    .execute(pool)
    .await
    .expect("seed refund");
}

/// A pending invoice's still-watched address - the case `get_active_watched_addresses_for_stores`
/// exists for, since it has no payment and so trips none of the other
/// cleanup queries (all scoped to expired/paid/cancelled invoices).
pub(crate) async fn seed_payment_option(pool: &PgPool, invoice: &str, address: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payment_options \
         (id, invoice_id, payment_method_id, chain_id, asset_type, asset_symbol, payment_address, amount) \
         VALUES ($1, $2, 'ETH-11155111', 'eip155:11155111', 'native', 'ETH', $3, 1)",
    )
    .bind(id)
    .bind(invoice)
    .bind(address)
    .execute(pool)
    .await
    .expect("seed payment option");
    id
}

pub(crate) async fn seed_watched_address(
    pool: &PgPool,
    invoice: &str,
    payment_option: Uuid,
    address: &str,
) {
    sqlx::query(
        "INSERT INTO watched_addresses \
         (invoice_id, payment_option_id, address, chain_id, expires_at) \
         VALUES ($1, $2, $3, 'eip155:11155111', now() + interval '1 hour')",
    )
    .bind(invoice)
    .bind(payment_option)
    .bind(address)
    .execute(pool)
    .await
    .expect("seed watched address");
}

pub(crate) async fn seed_payout(pool: &PgPool, store: Uuid) {
    sqlx::query(
        "INSERT INTO payouts (id, store_id, destination_address, chain_id, asset_symbol, amount) \
         VALUES ($1, $2, '0x0000000000000000000000000000000000000000', 'eip155:11155111', 'ETH', '1')",
    )
    .bind(Uuid::new_v4())
    .bind(store)
    .execute(pool)
    .await
    .expect("seed payout");
}

pub(crate) async fn cleanup(pool: &PgPool, users: &[Uuid]) {
    for user in users {
        let _ = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(pool)
            .await;
    }
}

/// A payout blocks a user's own cascading delete (`stores -> payouts` is not
/// `ON DELETE CASCADE`, see `data_service::account_deletion`), so a leftover
/// payout from a test that failed partway must be cleared before `cleanup`
/// can remove the user underneath it.
pub(crate) async fn clear_payouts_for_store(pool: &PgPool, store: Uuid) {
    let _ = sqlx::query("DELETE FROM payouts WHERE store_id = $1")
        .bind(store)
        .execute(pool)
        .await;
}

pub(crate) async fn clear_refunds_for_store(pool: &PgPool, store: Uuid) {
    let _ = sqlx::query("DELETE FROM refunds WHERE store_id = $1")
        .bind(store)
        .execute(pool)
        .await;
}

/// How many times the post-delete unwatch counter fired, read out of a
/// rendered metrics exposition.
///
/// A metric missing from the render never fired, which is genuinely zero. A
/// value that is present but unreadable is not - reporting that as zero is the
/// could-not-look-versus-found-nothing conflation this repository keeps paying
/// for - so it panics instead.
pub(crate) fn unwatch_failures(rendered: &str) -> u64 {
    const COUNTER: &str = "ethpayserver_unwatch_after_delete_failures_total";
    let mut found: Option<u64> = None;
    for line in rendered.lines() {
        // `# HELP`/`# TYPE` lines start with `#` and never match. A space
        // separates name from value; anything else is a longer metric name
        // that merely starts with this one.
        let Some(value) = line
            .strip_prefix(COUNTER)
            .and_then(|rest| rest.strip_prefix(' '))
        else {
            continue;
        };
        found = Some(
            value
                .trim()
                .parse()
                .unwrap_or_else(|e| panic!("could not read {COUNTER} from {line:?}: {e}")),
        );
    }
    found.unwrap_or(0)
}

pub(crate) fn admin_auth(id: Uuid) -> AdminAuth {
    AdminAuth(UserInfo {
        id: UserId(id),
        email: None,
        primary_wallet_address: None,
        created_at: chrono::Utc::now(),
        last_login_at: None,
        role: Role::ServerAdmin,
    })
}

/// A non-admin caller acting on their own account, for `delete_account`
/// (`DELETE /users/me`) rather than the admin routes. No email, matching
/// `seed_user` - `deletion_confirmation_for` falls back to the id in that
/// case, which is what these tests pass as `confirm`.
pub(crate) fn self_auth(id: Uuid) -> AuthenticatedUser {
    AuthenticatedUser(UserInfo {
        id: UserId(id),
        email: None,
        primary_wallet_address: None,
        created_at: chrono::Utc::now(),
        last_login_at: None,
        role: Role::User,
    })
}

pub(crate) fn app_state(data_service: Arc<PgDataService>) -> PgAppState<UnusedSessionService> {
    app_state_with_monitor(data_service, None)
}

pub(crate) fn app_state_with_monitor(
    data_service: Arc<PgDataService>,
    evm_monitor: Option<Arc<RedisEVMMonitor>>,
) -> PgAppState<UnusedSessionService> {
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        evm_monitor,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}
