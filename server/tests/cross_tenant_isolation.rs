#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Two merchants, A and B, each with their own store, invoice, payment,
//! wallet and API key. Every store-scoped read endpoint this file enumerates
//! (invoices and payments including list, by id and CSV export; stores
//! including by id, list, members, webhook config and token policy; wallets;
//! dashboard aggregates; payouts, refunds and webhook deliveries) is asked
//! for the other tenant's data, by id, by store filter, by the "all stores"
//! path, and authenticated with an API key instead of a session where that
//! axis applies, and must refuse. This is not a claim that literally every
//! handler under `server/src/api` is covered; it is the enumeration of the
//! store/tenant-scoped ones, grown each time a gap was found.
//!
//! This has shipped broken in both directions before: a nil-UUID `store_id`
//! that meant "every store" leaked one merchant's invoices and payments to
//! any authenticated caller, and - separately - the fix for that leak made
//! the "all stores" view admin-only, so a merchant asking for their own
//! stores with no filter got refused instead of an empty-looking answer.
//! Both are the same missing test: nobody asked "can A see B's row", in
//! either the leaking direction or the withholding one.
//!
//! A third shape is a key that carries more than its owner's scope rather
//! than a session that does: the stored row has no scope field narrower than
//! "everything its owner can do", so this schema can produce that in two
//! ways - a bearer-token path that resolves to the wrong owner or skips the
//! per-request tenant check a session goes through (the API-key tests below
//! re-run every session-tenancy assertion through the real key-hash lookup
//! for exactly that reason), and a key the active/expiry check should have
//! refused outright still authenticating, carrying its owner's full access
//! past a point it should never have reached (the revoked- and expired-key
//! tests at the end of that section, asserting the rejection itself rather
//! than what a successful call can reach). See the section comment above
//! them for the full argument.
//!
//! Calls handler functions directly against a real database, the same way
//! `plugin_invoice_creation_filter.rs` does: `AuthenticatedUser` and `State`
//! are plain data the extractors produce, and `server/src/api/**` handlers
//! are pinned to a concrete `State<PgAppState<A>>`, not a generic trait
//! object, so there is no way to run them against `InMemoryDataService`.
//!
//! Every `#[ignore]`'d test below needs `DATABASE_URL` and is not run by the
//! plain `cargo nextest run --workspace` pass. That is not a gap: CI's
//! "Integration tests" step already runs `cargo nextest run -p data-service
//! -p server --run-ignored only` against a real Postgres instance and gates
//! merges on it, the same lane `plugin_invoice_creation_filter.rs` relies on.
//! A test added to this file is exercised by that existing job with no
//! further wiring.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{Request as HttpRequest, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    ApiKey, ApiKeyId, ApiKeyRepository, Result as AuthResult, Session, SessionId, SessionService,
    Store, StoreRole, StoreRoleRepository, UserId, UserInfo, UserStore, UserStoreRepository,
};
use data_service::store_creation::StoreCreationWriter;
use data_service::{
    PayoutData, PayoutStatus, PayoutWriter, PgDataService, RefundData, RefundStatus, RefundWriter,
    StoreWebhookWriter, UpsertDeliveryParams, WalletWriter, WebhookDeliveryStatus,
    WebhookDeliveryWriter,
};
use payserver_plugin_api::PluginId;
use payserver_plugin_host::{PageHost, PageRenderError, PageRenderer};
use rates::NoOpRateProvider;
use server::api::AuthenticatedUser;
use server::api::stores::{SetStoreWalletRequest, StoreWalletResult};
use server::services::RedisEVMMonitor;
use server::services::plugins::{PageElement, PageRequest, Viewer};
use server::state::PgAppState;
use types::{
    AssetType, ChainId, InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, NAMESPACE_EIP155,
    PaymentData, PaymentWriter,
};

/// Not exercised: every handler under test reads only the already-
/// authenticated `UserInfo` this file constructs directly, or the API-key
/// path, which never calls back into session management either.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by the handlers under test")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by the handlers under test")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by the handlers under test")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by the handlers under test")
    }
}

/// `None` means "no `DATABASE_URL`, intentionally skipped" - the only case
/// that may pass silently. A `DATABASE_URL` that fails to connect is not the
/// same thing and must not collapse into the same silent `None`: that would
/// turn a broken or misconfigured CI database into every test in this file
/// reporting "passed" having run zero assertions, exactly the "test that
/// cannot fail" shape this suite exists to avoid.
async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .unwrap_or_else(|e| panic!("DATABASE_URL is set but the pool failed to connect: {e}"));
    Some(PgDataService::new(pool))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    // `kdf_params`/`encrypted_symmetric_key` need to deserialize into real
    // `crypto::KdfParams`/`EncryptedBlob` shapes (not just be valid JSON) -
    // the API-key auth path resolves the owning user via `UserRepository::
    // get_user`, which does that deserialization, unlike every other test
    // in this crate that hands `AuthenticatedUser` a `UserInfo` directly and
    // never reads this row back.
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, \
         '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"AAAA\"}'::jsonb, \
         '{\"ciphertext\":\"AAAA\",\"iv\":\"AAAA\",\"mac\":\"AAAA\"}'::jsonb, \
         'h', 'passkey:' || $1::text)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

fn user_info(id: Uuid) -> UserInfo {
    user_info_with_role(id, auth::Role::User)
}

fn user_info_with_role(id: Uuid, role: auth::Role) -> UserInfo {
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

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// One merchant's full footprint: a store, an invoice on it, a payment on
/// that invoice, an account wallet, and an API key.
struct Tenant {
    user_id: Uuid,
    store: Store,
    invoice: InvoiceData,
    payment_id: Uuid,
    wallet: data_service::Wallet,
    api_key_raw: String,
}

async fn seed_tenant(pg: &PgDataService, label: &str) -> Tenant {
    let user_id = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{label}-{}", Uuid::new_v4()), UserId(user_id));
    pg.create_store_owned_by(&store, UserId(user_id))
        .await
        .expect("seed store owned by user");

    let invoice = InvoiceData {
        id: InvoiceId::new(),
        store_id: types::StoreId(store.id.0),
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: "100.00".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(pg, &invoice)
        .await
        .expect("seed invoice");

    let payment = PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice.id.clone(),
        payment_option_id: None,
        chain_id: ChainId::evm(11155111),
        asset_type: AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
        block_number: Some(1),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: None,
        reorged: false,
        extra: None,
        credited_amount: None,
        rate_used: None,
        rate_applied_at: None,
    };
    PaymentWriter::upsert(pg, &payment)
        .await
        .expect("seed payment");

    let wallet = WalletWriter::create_wallet(
        pg,
        user_id,
        NAMESPACE_EIP155,
        &format!("xpub-fake-{}", Uuid::new_v4()),
        None,
    )
    .await
    .expect("seed wallet");

    let api_key_raw = format!("ak_test_{}", Uuid::new_v4());
    ApiKeyRepository::create_api_key(
        pg,
        &ApiKey {
            id: ApiKeyId::new(),
            user_id: UserId(user_id),
            name: "cross-tenant test key".to_string(),
            key_hash: sha256_hex(&api_key_raw),
            key_prefix: "ak_test_****".to_string(),
            is_active: true,
            created_at: Utc::now(),
            last_used_at: None,
            expires_at: None,
        },
    )
    .await
    .expect("seed api key");

    Tenant {
        user_id,
        store,
        invoice,
        payment_id: payment.id,
        wallet,
        api_key_raw,
    }
}

/// Switches a tenant's role on their own store to a one-off role carrying
/// exactly `permission`, for the one endpoint (`list_store_members`) whose
/// permission string none of the seeded default roles - Owner included -
/// actually grant. Without this, a positive control against that endpoint
/// would fail for every caller, not just a foreign one, which is a real gap
/// in this repo's default roles but not what this suite exists to prove.
async fn grant_store_permission(pg: &PgDataService, tenant: &Tenant, permission: &str) {
    let role = StoreRole::new(
        auth::StoreId(tenant.store.id.0),
        "cross-tenant-test-role",
        vec![permission.to_string()],
    );
    StoreRoleRepository::create_store_role(pg, &role)
        .await
        .expect("create a role carrying the permission under test");
    UserStoreRepository::update_user_store(
        pg,
        &UserStore::new(
            UserId(tenant.user_id),
            auth::StoreId(tenant.store.id.0),
            role.id,
        ),
    )
    .await
    .expect("assign the test role to the tenant's own store membership");
}

/// A payout on `store`, unrelated to any real invoice - the payout endpoints
/// under test only ever check the payout's own `store_id`, never its
/// `invoice_ids`.
async fn seed_payout(pg: &PgDataService, store: &Store) -> Uuid {
    let payout = PayoutData {
        id: Uuid::new_v4(),
        store_id: types::StoreId(store.id.0),
        invoice_ids: vec![],
        destination_address: format!("0x{:040x}", Uuid::new_v4().as_u128()),
        chain_id: ChainId::evm(11155111),
        asset_type: "native".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        amount: "1000000000000000000".to_string(),
        tx_hash: None,
        status: PayoutStatus::Pending,
        fee_amount: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    };
    PayoutWriter::create_payout(pg, &payout)
        .await
        .expect("seed payout");
    payout.id
}

/// A refund on the tenant's own invoice and payment.
async fn seed_refund(pg: &PgDataService, tenant: &Tenant) -> Uuid {
    let refund = RefundData {
        id: Uuid::new_v4(),
        invoice_id: tenant.invoice.id.clone(),
        payment_id: tenant.payment_id,
        store_id: types::StoreId(tenant.store.id.0),
        to_address: format!("0x{:040x}", Uuid::new_v4().as_u128()),
        chain_id: ChainId::evm(11155111),
        asset_type: "native".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        amount: "500000000000000000".to_string(),
        tx_hash: None,
        status: RefundStatus::Pending,
        fee_amount: None,
        reason: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    };
    RefundWriter::create_refund(pg, &refund)
        .await
        .expect("seed refund");
    refund.id
}

/// A delivered webhook delivery against the tenant's own invoice, behind a
/// webhook configured for the tenant's store.
async fn seed_webhook_delivery(pg: &PgDataService, tenant: &Tenant) -> Uuid {
    let webhook = StoreWebhookWriter::upsert_webhook(
        pg,
        tenant.store.id.0,
        "https://example.com/webhook",
        "secret",
        true,
    )
    .await
    .expect("seed store webhook");

    let delivery_id = Uuid::new_v4();
    WebhookDeliveryWriter::upsert_delivery(
        pg,
        UpsertDeliveryParams {
            id: delivery_id,
            store_webhook_id: webhook.id,
            invoice_id: tenant.invoice.id.0.clone(),
            event_type: "invoice_expired".to_string(),
            status: WebhookDeliveryStatus::Delivered,
            attempts: 1,
            max_attempts: 7,
            last_error: None,
            payload: serde_json::json!({"event_type": "invoice_expired"}),
        },
    )
    .await
    .expect("seed webhook delivery");
    delivery_id
}

/// Runs a raw bearer token through the same extractor a real request would,
/// so the API-key branch of `validate_session` (hashing, active/expiry
/// checks, resolving the owning user) is what authenticates - not a
/// hand-built `UserInfo` that would skip it entirely.
async fn authenticate_via_bearer<A: SessionService + 'static>(
    state: &PgAppState<A>,
    raw_token: &str,
) -> AuthenticatedUser {
    let request = HttpRequest::builder()
        .header("authorization", format!("Bearer {raw_token}"))
        .body(())
        .expect("build request");
    let (mut parts, ()) = request.into_parts();
    AuthenticatedUser::from_request_parts(&mut parts, state)
        .await
        .expect("bearer token authenticates")
}

fn status_of<T>(result: Result<T, server::api::ApiErr>) -> StatusCode {
    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => e.into_response().status(),
    }
}

// ============================================================================
// Invoices
// ============================================================================

#[tokio::test]
#[ignore]
async fn list_invoices_with_no_store_id_shows_only_the_callers_own() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: None,
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a merchant with no store filter must see their own invoices, not be refused");

    let ids: Vec<String> = result.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(
        ids.contains(&a.invoice.id.0),
        "A's own invoice must be visible with no store filter"
    );
    assert!(
        !ids.contains(&b.invoice.id.0),
        "B's invoice leaked into A's unfiltered 'all stores' view"
    );
}

#[tokio::test]
#[ignore]
async fn list_invoices_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(b.store.id.0),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "A must not be able to list B's store by naming its id directly"
    );

    // Positive control: without this, an endpoint that refuses every explicit
    // store_id, including the caller's own, would pass the assertion above
    // for the wrong reason.
    let own = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(a.store.id.0),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("A must be able to list A's own store by naming its id directly");
    let ids: Vec<String> = own.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(
        ids.contains(&a.invoice.id.0),
        "A's own invoice must be visible when A names A's own store id"
    );
}

/// The literal historical bug: a nil UUID once took a different code path
/// than "no filter" or "a real foreign store id" and skipped both the
/// membership check and the `WHERE store_id` clause, handing a merchant
/// every invoice on the server. A nil `store_id` must be refused exactly
/// like any other store A does not belong to, not treated as "everything".
#[tokio::test]
#[ignore]
async fn list_invoices_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(Uuid::nil()),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "a nil store_id must not be treated as 'every store'"
    );
}

/// Contrasts the two membership-only tests above: a `ServerAdmin` asking for
/// the same store filter as a `User` must not stop at the caller's own
/// stores. If this bypass ever silently loosened to cover `Role::User` too,
/// the earlier tests would already fail; this test is what proves the
/// bypass is real for the role that is supposed to have it.
#[tokio::test]
#[ignore]
async fn list_invoices_with_no_store_id_as_server_admin_sees_every_tenant() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_invoices(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: None,
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a server admin must be able to list with no store filter");

    let ids: Vec<String> = result.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(
        ids.contains(&a.invoice.id.0),
        "an admin's unfiltered view must still include their own invoice"
    );
    assert!(
        ids.contains(&b.invoice.id.0),
        "an admin's unfiltered view must reach every tenant, not just their own"
    );
}

#[tokio::test]
#[ignore]
async fn get_invoice_by_id_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to fetch B's invoice by id"
    );

    // Positive control: the admin test below proves the admin bypass works,
    // but says nothing about the ownership branch a regular merchant goes
    // through. Without this, an endpoint that refused every non-admin caller
    // regardless of ownership would still pass the assertion above for the
    // wrong reason.
    let own = server::api::invoices::get_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to fetch A's own invoice by id");
    assert_eq!(own.id, a.invoice.id.0);
}

/// The positive control for the test above: the same cross-tenant request,
/// with the caller's role swapped to `ServerAdmin`, must succeed. Without
/// this, a bug that made every caller FORBIDDEN regardless of role would
/// still pass the negative test.
#[tokio::test]
#[ignore]
async fn get_invoice_across_tenants_is_permitted_for_a_server_admin() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.invoice.id.0.clone()),
    )
    .await
    .expect("a server admin must be able to fetch any tenant's invoice by id");

    assert_eq!(result.id, b.invoice.id.0);
}

#[tokio::test]
#[ignore]
async fn get_invoice_payments_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list B's invoice's payments"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::invoices::get_invoice_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to list payments on A's own invoice");
    assert!(own.iter().any(|p| p.id == a.payment_id.to_string()));
}

#[tokio::test]
#[ignore]
async fn get_invoice_status_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_invoice_status(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to read B's invoice status"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::invoices::get_invoice_status(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to read A's own invoice status");
    assert_eq!(own.id, a.invoice.id.0);
}

// ============================================================================
// Payments
// ============================================================================

#[tokio::test]
#[ignore]
async fn list_payments_with_no_store_id_shows_only_the_callers_own() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: None,
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a merchant with no store filter must see their own payments, not be refused");

    let ids: Vec<String> = result.payments.iter().map(|p| p.id.clone()).collect();
    assert!(
        ids.contains(&a.payment_id.to_string()),
        "A's own payment must be visible with no store filter"
    );
    assert!(
        !ids.contains(&b.payment_id.to_string()),
        "B's payment leaked into A's unfiltered 'all stores' view"
    );
}

#[tokio::test]
#[ignore]
async fn list_payments_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(b.store.id.0),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "A must not be able to list B's payments by naming its store id directly"
    );

    // Positive control: without this, an endpoint that refuses every explicit
    // store_id, including the caller's own, would pass the assertion above
    // for the wrong reason.
    let own = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(a.store.id.0),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("A must be able to list A's own payments by naming its store id directly");
    let ids: Vec<String> = own.payments.iter().map(|p| p.id.clone()).collect();
    assert!(
        ids.contains(&a.payment_id.to_string()),
        "A's own payment must be visible when A names A's own store id"
    );
}

/// The payments side of the same nil-UUID bug the invoice test above guards
/// against: a nil `store_id` must be refused, not read as "every store".
#[tokio::test]
#[ignore]
async fn list_payments_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(Uuid::nil()),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "a nil store_id must not be treated as 'every store'"
    );
}

/// The payments side of `list_invoices_with_no_store_id_as_server_admin_sees_every_tenant`:
/// a `ServerAdmin` with no store filter must reach every tenant's payments,
/// not just their own.
#[tokio::test]
#[ignore]
async fn list_payments_with_no_store_id_as_server_admin_sees_every_tenant() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::list_payments(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: None,
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("a server admin must be able to list payments with no store filter");

    let ids: Vec<String> = result.payments.iter().map(|p| p.id.clone()).collect();
    assert!(
        ids.contains(&a.payment_id.to_string()),
        "an admin's unfiltered view must still include their own payment"
    );
    assert!(
        ids.contains(&b.payment_id.to_string()),
        "an admin's unfiltered view must reach every tenant, not just their own"
    );
}

#[tokio::test]
#[ignore]
async fn get_payment_by_id_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::get_payment(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.payment_id),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch B's payment by id"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::invoices::get_payment(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.payment_id),
    )
    .await
    .expect("A must be able to fetch A's own payment by id");
    assert_eq!(own.id, a.payment_id.to_string());
}

// ============================================================================
// Stores
// ============================================================================

#[tokio::test]
#[ignore]
async fn get_store_by_id_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::get_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to fetch B's store by id"
    );

    // Positive control: without this, an endpoint that refuses regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::stores::get_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to fetch A's own store by id");
    assert_eq!(own.id, a.store.id.0);
}

#[tokio::test]
#[ignore]
async fn list_stores_never_includes_another_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result =
        server::api::stores::list_stores(AuthenticatedUser(user_info(a.user_id)), State(state))
            .await
            .expect("listing one's own stores must succeed");

    let ids: Vec<Uuid> = result.iter().map(|s| s.id).collect();
    assert!(ids.contains(&a.store.id.0), "A's own store must be listed");
    assert!(
        !ids.contains(&b.store.id.0),
        "B's store leaked into A's store list"
    );
}

// ============================================================================
// Wallets
// ============================================================================

#[tokio::test]
#[ignore]
async fn wallets_are_scoped_to_the_owning_account() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let listed = server::api::stores::list_wallets(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
    )
    .await
    .expect("listing one's own wallets must succeed");
    assert!(
        listed.iter().any(|w| w.id == a.wallet.id),
        "A's own wallet must be listed"
    );
    assert!(
        !listed.iter().any(|w| w.id == b.wallet.id),
        "B's wallet leaked into A's wallet list"
    );

    let result = server::api::stores::get_wallet_by_id(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.wallet.id),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch B's wallet by id"
    );

    // Positive control: without this, `get_wallet_by_id` refusing every
    // caller, including one asking about their own wallet, would pass the
    // assertion above for the wrong reason.
    let own = server::api::stores::get_wallet_by_id(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.wallet.id),
    )
    .await
    .expect("A must be able to fetch A's own wallet by id");
    assert_eq!(own.id, a.wallet.id);
}

#[tokio::test]
#[ignore]
async fn store_wallet_endpoints_refuse_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::stores::get_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
        Query(server::api::stores::StoreWalletQuery {
            namespace: None,
            // The bare form: these tests are about who may read a store's
            // wallet at all, not about which wallet a payment method
            // resolves to. Method-scoped resolution has its own tests.
            payment_method_id: None,
        }),
    )
    .await;
    assert_eq!(
        get_result.err(),
        Some(StatusCode::FORBIDDEN),
        "A must not be able to read B's store wallet"
    );

    let configure_result = server::api::stores::configure_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
        axum::Json(SetStoreWalletRequest {
            wallet_id: a.wallet.id,
        }),
    )
    .await;
    assert_eq!(
        configure_result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to pin a wallet onto B's store"
    );

    // Positive control: without this, `get_store_wallet` refusing every
    // caller, including one asking about their own store, would pass the
    // assertion above for the wrong reason.
    let own = server::api::stores::get_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
        Query(server::api::stores::StoreWalletQuery {
            namespace: None,
            // The bare form: these tests are about who may read a store's
            // wallet at all, not about which wallet a payment method
            // resolves to. Method-scoped resolution has its own tests.
            payment_method_id: None,
        }),
    )
    .await
    .expect("A must be able to read A's own store wallet");
    let StoreWalletResult::Bare(own) = own else {
        panic!("bare form (no payment_method_id) must resolve to StoreWalletResult::Bare");
    };
    assert_eq!(own.wallet.id, a.wallet.id);
}

#[tokio::test]
#[ignore]
async fn store_wallet_override_refuses_a_wallet_from_another_account() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    // A owns this store, so the permission check passes; the repository is
    // what must refuse pointing it at a wallet from B's account.
    let result = server::api::stores::configure_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(a.store.id.0),
        axum::Json(SetStoreWalletRequest {
            wallet_id: b.wallet.id,
        }),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A's own store must not be pinnable to B's wallet"
    );

    // Positive control: without this, `configure_store_wallet` refusing
    // every wallet id, including the caller's own, would pass the
    // assertion above for the wrong reason.
    let own = server::api::stores::configure_store_wallet(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
        axum::Json(SetStoreWalletRequest {
            wallet_id: a.wallet.id,
        }),
    )
    .await
    .expect("A must be able to pin A's own store to A's own wallet");
    assert_eq!(own.wallet.id, a.wallet.id);
}

// ============================================================================
// Payouts, refunds, and webhook deliveries: the same store-membership shape
// as invoices and payments (a path id checked with `get_user_store`, then the
// row itself matched to that store), so the same nil/foreign/admin questions
// apply and had no coverage at all before this test.
// ============================================================================

#[tokio::test]
#[ignore]
async fn payout_endpoints_refuse_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_payout = seed_payout(&pg, &a.store).await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path((b.store.id.0, b_payout)),
    )
    .await;
    assert_eq!(
        get_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch a payout on B's store"
    );

    let list_result = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        list_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list payouts on B's store"
    );

    // Positive control: without this, both endpoints refusing every caller,
    // including one asking about their own store, would pass the assertions
    // above for the wrong reason.
    let own = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path((a.store.id.0, a_payout)),
    )
    .await
    .expect("A must be able to fetch a payout on A's own store");
    assert_eq!(own.id, a_payout);

    let own_list = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to list payouts on A's own store");
    assert!(own_list.payouts.iter().any(|p| p.id == a_payout));
}

/// The "id from B passed directly to a detail endpoint" case: A names A's own
/// store, so the membership gate passes, but supplies B's payout id. The
/// membership check alone must not be enough - the payout itself has to be
/// matched to the store named in the path, the same as every other
/// `*_for_store` lookup in this file.
#[tokio::test]
#[ignore]
async fn get_payout_refuses_another_tenants_payout_even_via_the_callers_own_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path((a.store.id.0, b_payout)),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to fetch B's payout by naming A's own store and B's payout id"
    );
}

#[tokio::test]
#[ignore]
async fn list_refunds_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_refund = seed_refund(&pg, &a).await;
    let _b_refund = seed_refund(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::refunds::list_refunds(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list refunds on B's invoice"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::refunds::list_refunds(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to list refunds on A's own invoice");
    assert!(own.iter().any(|r| r.id == a_refund));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_invoice_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list webhook deliveries on B's invoice"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::webhook_deliveries::list_deliveries_for_invoice(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("A must be able to list webhook deliveries on A's own invoice");
    assert!(own.deliveries.iter().any(|d| d.id == a_delivery));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_store_across_tenants_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "A must not be able to list webhook deliveries on B's store"
    );

    // Positive control: without this, an endpoint that 404s regardless of
    // caller would pass the assertion above for the wrong reason.
    let own = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to list webhook deliveries on A's own store");
    assert!(own.deliveries.iter().any(|d| d.id == a_delivery));
}

/// The nil-`store_id` bug shape, for the two endpoints in this section keyed
/// directly by a `store_id` path segment. `list_refunds` and
/// `list_deliveries_for_invoice` are keyed by invoice id (a string, not a
/// UUID) instead, so there is no nil-`store_id` case to construct for them -
/// the admin-bypass tests below cover those two.
#[tokio::test]
#[ignore]
async fn payout_endpoints_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let a_payout = seed_payout(&pg, &a.store).await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path((Uuid::nil(), a_payout)),
    )
    .await;
    assert_eq!(
        get_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a nil store_id must not be treated as 'every store'"
    );

    let list_result = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(Uuid::nil()),
    )
    .await;
    assert_eq!(
        list_result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a nil store_id must not be treated as 'every store'"
    );
}

/// The admin-bypass side of the same shape: a `ServerAdmin` is not a member
/// of either tenant's store, yet must still reach both - the direction where
/// an over-narrow membership check would wrongly refuse the one role that is
/// supposed to see everything.
#[tokio::test]
#[ignore]
async fn payout_endpoints_as_server_admin_reach_every_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let state = app_state(Arc::new(pg));

    let get_result = server::api::payouts::get_payout(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state.clone()),
        Path((b.store.id.0, b_payout)),
    )
    .await
    .expect("a server admin must be able to fetch another tenant's payout");
    assert_eq!(get_result.id, b_payout);

    let list_result = server::api::payouts::list_payouts(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.store.id.0),
    )
    .await
    .expect("a server admin must be able to list another tenant's payouts");
    assert!(list_result.payouts.iter().any(|p| p.id == b_payout));
}

#[tokio::test]
#[ignore]
async fn list_refunds_as_server_admin_reaches_another_tenants_invoice() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_refund = seed_refund(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::refunds::list_refunds(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.invoice.id.0.clone()),
    )
    .await
    .expect("a server admin must be able to list another tenant's refunds");
    assert!(result.iter().any(|r| r.id == b_refund));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_invoice_as_server_admin_reaches_another_tenants_invoice() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_invoice(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.invoice.id.0.clone()),
    )
    .await
    .expect("a server admin must be able to list another tenant's webhook deliveries");
    assert!(result.deliveries.iter().any(|d| d.id == b_delivery));
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_store_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(Uuid::nil()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "a nil store_id must not be treated as 'every store'"
    );
}

#[tokio::test]
#[ignore]
async fn list_deliveries_for_store_as_server_admin_reaches_every_tenants_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        AuthenticatedUser(user_info_with_role(a.user_id, auth::Role::ServerAdmin)),
        State(state),
        Path(b.store.id.0),
    )
    .await
    .expect("a server admin must be able to list another tenant's webhook deliveries by store");
    assert!(result.deliveries.iter().any(|d| d.id == b_delivery));
}

// ============================================================================
// API keys: the same tenancy boundary, reached through the other auth path
//
// The stored row (id, owner, name, hash, prefix, active flag, expiry) has no
// scope field narrower than "everything its owner can do" - there is no
// per-key store binding and no permission set to shrink. So "a key carrying
// more than its owner's scope" has two shapes this schema can actually
// produce, and this section covers both: a bearer-token path that resolves
// to the wrong owner, or skips the per-request tenant check a session goes
// through (the tests below, re-running every session-tenancy assertion
// through `authenticate_via_bearer` - the real key-hash lookup and owner
// resolution, not a hand-built session); and a key the active/expiry check
// should have already refused authenticating anyway, carrying its owner's
// full access past a point it should never have reached (the revoked- and
// expired-key tests at the end of this section, which go through
// `AuthenticatedUser::from_request_parts` directly to assert the rejection
// itself, not just what a successful call can reach). If a narrower per-key
// scope is ever added, it needs its own tests here.
//
// What this section does NOT claim: it is not a reproduction of any specific
// tracked defect, open or otherwise - this file has no way to read a tracker
// and doesn't try to. It is an exhaustive list of the scope-violation shapes
// *this schema* can structurally produce today. If a real key-scope bug
// turns out to need a shape this schema cannot express (e.g. a key legitimately
// narrower than its owner, reaching beyond that narrower grant), no test here
// proves or disproves it, and closing that would need a schema change first -
// a new column and a check against it, then a test here for that check.
// ============================================================================

#[tokio::test]
#[ignore]
async fn an_api_key_is_bound_to_its_owners_tenancy_same_as_a_session() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    assert_eq!(
        a_via_key.0.id,
        UserId(a.user_id),
        "the api key must resolve to its own owner"
    );

    let result = server::api::invoices::get_invoice(
        a_via_key,
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "an API key must not reach another tenant's invoice any more than a session can"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let listed = server::api::invoices::list_invoices(
        a_via_key,
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: None,
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("an api key with no store filter must see its owner's invoices");

    let ids: Vec<String> = listed.invoices.iter().map(|i| i.id.clone()).collect();
    assert!(ids.contains(&a.invoice.id.0));
    assert!(
        !ids.contains(&b.invoice.id.0),
        "an API key's unfiltered listing must not include another tenant's invoice"
    );
}

/// The payment side of the test above. An API key that carried more than its
/// owner's scope would be a distinct bug from session tenancy, so every
/// payment-reading endpoint - not just invoices - needs its own
/// API-key-authenticated check, not just the session-based ones above.
#[tokio::test]
#[ignore]
async fn an_api_key_cannot_reach_another_tenants_payments() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result =
        server::api::invoices::get_payment(a_via_key, State(state.clone()), Path(b.payment_id))
            .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not fetch another tenant's payment by id"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::invoices::get_invoice_payments(
        a_via_key,
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not list another tenant's invoice's payments"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::invoices::get_invoice_status(
        a_via_key,
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not read another tenant's invoice status"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let listed = server::api::invoices::list_payments(
        a_via_key,
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: None,
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("an api key with no store filter must see its owner's payments");

    let ids: Vec<String> = listed.payments.iter().map(|p| p.id.clone()).collect();
    assert!(ids.contains(&a.payment_id.to_string()));
    assert!(
        !ids.contains(&b.payment_id.to_string()),
        "an API key's unfiltered payment listing must not include another tenant's payment"
    );
}

/// The wallet side of the same boundary: a key authenticates as its owner,
/// and the owner's wallet scoping applies exactly as it does to a session.
#[tokio::test]
#[ignore]
async fn an_api_key_cannot_reach_another_tenants_wallets() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let listed = server::api::stores::list_wallets(a_via_key, State(state.clone()))
        .await
        .expect("listing one's own wallets via an api key must succeed");
    assert!(
        listed.iter().any(|w| w.id == a.wallet.id),
        "A's own wallet must be listed via an api key"
    );
    assert!(
        !listed.iter().any(|w| w.id == b.wallet.id),
        "B's wallet leaked into A's api-key-authenticated wallet list"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result =
        server::api::stores::get_wallet_by_id(a_via_key, State(state.clone()), Path(b.wallet.id))
            .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not fetch another tenant's wallet by id"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::stores::get_store_wallet(
        a_via_key,
        State(state.clone()),
        Path(b.store.id.0),
        Query(server::api::stores::StoreWalletQuery {
            namespace: None,
            // The bare form: these tests are about who may read a store's
            // wallet at all, not about which wallet a payment method
            // resolves to. Method-scoped resolution has its own tests.
            payment_method_id: None,
        }),
    )
    .await;
    assert_eq!(
        result.err(),
        Some(StatusCode::FORBIDDEN),
        "an API key must not read another tenant's store wallet"
    );

    // Positive control: without this, `get_store_wallet` refusing every
    // caller, API-key included, would pass the assertion above for the
    // wrong reason.
    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own = server::api::stores::get_store_wallet(
        a_via_key,
        State(state),
        Path(a.store.id.0),
        Query(server::api::stores::StoreWalletQuery {
            namespace: None,
            // The bare form: these tests are about who may read a store's
            // wallet at all, not about which wallet a payment method
            // resolves to. Method-scoped resolution has its own tests.
            payment_method_id: None,
        }),
    )
    .await
    .expect("an API key must be able to read its owner's own store wallet");
    let StoreWalletResult::Bare(own) = own else {
        panic!("bare form (no payment_method_id) must resolve to StoreWalletResult::Bare");
    };
    assert_eq!(own.wallet.id, a.wallet.id);
}

#[tokio::test]
#[ignore]
async fn an_api_key_cannot_reach_another_tenants_stores() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result =
        server::api::stores::get_store(a_via_key, State(state.clone()), Path(b.store.id.0)).await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "an API key must not fetch another tenant's store by id"
    );

    // Positive control: without this, `get_store` refusing every caller,
    // API-key included, would pass the assertion above for the wrong
    // reason.
    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own = server::api::stores::get_store(a_via_key, State(state.clone()), Path(a.store.id.0))
        .await
        .expect("an API key must be able to fetch its owner's own store by id");
    assert_eq!(own.id, a.store.id.0);

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let listed = server::api::stores::list_stores(a_via_key, State(state))
        .await
        .expect("listing one's own stores via an api key must succeed");
    let ids: Vec<Uuid> = listed.iter().map(|s| s.id).collect();
    assert!(
        ids.contains(&a.store.id.0),
        "A's own store must be listed via an api key"
    );
    assert!(
        !ids.contains(&b.store.id.0),
        "B's store leaked into A's api-key-authenticated store list"
    );
}

/// The payout/refund/webhook-delivery side of the same boundary: an API key
/// resolves to its owner, and the owner's store-membership scoping applies
/// exactly as it does to a session.
#[tokio::test]
#[ignore]
async fn an_api_key_cannot_reach_another_tenants_payouts_refunds_or_deliveries() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let b_payout = seed_payout(&pg, &b.store).await;
    let _a_refund = seed_refund(&pg, &a).await;
    let _b_refund = seed_refund(&pg, &b).await;
    let _a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::payouts::get_payout(
        a_via_key,
        State(state.clone()),
        Path((b.store.id.0, b_payout)),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not fetch a payout on another tenant's store"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result =
        server::api::payouts::list_payouts(a_via_key, State(state.clone()), Path(b.store.id.0))
            .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not list payouts on another tenant's store"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::refunds::list_refunds(
        a_via_key,
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not list refunds on another tenant's invoice"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::webhook_deliveries::list_deliveries_for_invoice(
        a_via_key,
        State(state.clone()),
        Path(b.invoice.id.0.clone()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not list webhook deliveries on another tenant's invoice"
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let result = server::api::webhook_deliveries::list_deliveries_for_store(
        a_via_key,
        State(state),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::NOT_FOUND,
        "an API key must not list webhook deliveries on another tenant's store"
    );
}

/// Positive controls for the test above: without these, any of its five
/// endpoints refusing every caller, API-key included, would pass its
/// negative assertion for the wrong reason.
#[tokio::test]
#[ignore]
async fn an_api_keys_own_payouts_refunds_and_deliveries_remain_reachable() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let a_payout = seed_payout(&pg, &a.store).await;
    let a_refund = seed_refund(&pg, &a).await;
    let a_delivery = seed_webhook_delivery(&pg, &a).await;
    let state = app_state(Arc::new(pg));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own_payout = server::api::payouts::get_payout(
        a_via_key,
        State(state.clone()),
        Path((a.store.id.0, a_payout)),
    )
    .await
    .expect("an API key must be able to fetch its owner's own payout");
    assert_eq!(own_payout.id, a_payout);

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own_payouts =
        server::api::payouts::list_payouts(a_via_key, State(state.clone()), Path(a.store.id.0))
            .await
            .expect("an API key must be able to list its owner's own payouts");
    assert!(own_payouts.payouts.iter().any(|p| p.id == a_payout));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own_refunds = server::api::refunds::list_refunds(
        a_via_key,
        State(state.clone()),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("an API key must be able to list its owner's own refunds");
    assert!(own_refunds.iter().any(|r| r.id == a_refund));

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own_invoice_deliveries = server::api::webhook_deliveries::list_deliveries_for_invoice(
        a_via_key,
        State(state.clone()),
        Path(a.invoice.id.0.clone()),
    )
    .await
    .expect("an API key must be able to list its owner's own invoice's webhook deliveries");
    assert!(
        own_invoice_deliveries
            .deliveries
            .iter()
            .any(|d| d.id == a_delivery)
    );

    let a_via_key = authenticate_via_bearer(&state, &a.api_key_raw).await;
    let own_store_deliveries = server::api::webhook_deliveries::list_deliveries_for_store(
        a_via_key,
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("an API key must be able to list its owner's own store's webhook deliveries");
    assert!(
        own_store_deliveries
            .deliveries
            .iter()
            .any(|d| d.id == a_delivery)
    );
}

// The scope-less schema means the tenancy tests above are the whole story for
// "does a key ever reach more than its owner's data" - but there is a second
// way a key can carry more than it should: the active/expiry check that gates
// the hash lookup silently not applying. These two hit that mechanism
// directly, going through `AuthenticatedUser::from_request_parts` (not the
// `authenticate_via_bearer` helper, which unwraps and would panic on the
// rejection under test) against a key whose row is revoked, or expired,
// exactly the way an operator revocation or a client-set TTL would leave it.

#[tokio::test]
#[ignore]
async fn a_revoked_api_key_no_longer_authenticates() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let key = ApiKeyRepository::get_api_key_by_hash(&pg, &sha256_hex(&a.api_key_raw))
        .await
        .expect("look up the seeded key")
        .expect("seeded key exists");
    ApiKeyRepository::revoke_api_key(&pg, key.id)
        .await
        .expect("revoke it the same way an operator revocation would");
    let state = app_state(Arc::new(pg));

    let request = HttpRequest::builder()
        .header("authorization", format!("Bearer {}", a.api_key_raw))
        .body(())
        .expect("build request");
    let (mut parts, ()) = request.into_parts();
    let result = AuthenticatedUser::from_request_parts(&mut parts, &state).await;
    assert_eq!(
        result.err().map(|(status, _)| status),
        Some(StatusCode::UNAUTHORIZED),
        "a revoked key's hash still matches its row; the active flag, not the \
         hash lookup, is what must refuse it"
    );
}

#[tokio::test]
#[ignore]
async fn an_expired_api_key_no_longer_authenticates() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;

    let expired_raw = format!("ak_test_{}", Uuid::new_v4());
    ApiKeyRepository::create_api_key(
        &pg,
        &ApiKey {
            id: ApiKeyId::new(),
            user_id: UserId(a.user_id),
            name: "expired cross-tenant test key".to_string(),
            key_hash: sha256_hex(&expired_raw),
            key_prefix: "ak_test_****".to_string(),
            is_active: true,
            created_at: Utc::now() - chrono::Duration::hours(2),
            last_used_at: None,
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        },
    )
    .await
    .expect("seed an already-expired api key");
    let state = app_state(Arc::new(pg));

    let request = HttpRequest::builder()
        .header("authorization", format!("Bearer {expired_raw}"))
        .body(())
        .expect("build request");
    let (mut parts, ()) = request.into_parts();
    let result = AuthenticatedUser::from_request_parts(&mut parts, &state).await;
    assert_eq!(
        result.err().map(|(status, _)| status),
        Some(StatusCode::UNAUTHORIZED),
        "an expired key's hash still matches its row; the expiry check, not \
         the hash lookup, is what must refuse it"
    );
}

// ============================================================================
// CSV export: builds its filter through the same `verify_store_access_for_query`
// guard as `list_invoices`/`list_payments`, but is a separate handler and a
// separate response path (a streamed file, not JSON), so the guard being
// wired to the list endpoint proves nothing about the export one.
// ============================================================================

#[tokio::test]
#[ignore]
async fn export_invoices_csv_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_invoices_csv(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(b.store.id.0),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;
    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "A must not be able to export B's store's invoices by naming its id directly"
    );

    // Positive control: without this, an endpoint that refuses every explicit
    // store_id, including the caller's own, would pass the assertion above
    // for the wrong reason.
    let own = server::api::invoices::export_invoices_csv(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(a.store.id.0),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("A must be able to export A's own store's invoices by naming its id directly");
    assert_eq!(own.status(), StatusCode::OK);
}

/// The same nil-UUID regression `list_invoices` guards against (see above),
/// but for the export handler's own copy of the store-access check.
#[tokio::test]
#[ignore]
async fn export_invoices_csv_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_invoices_csv(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListInvoicesQuery {
            store_id: Some(Uuid::nil()),
            status: None,
            currency: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "a nil store_id must not be treated as 'every store'"
    );
}

#[tokio::test]
#[ignore]
async fn export_payments_csv_with_another_tenants_store_id_is_refused() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_payments_csv(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(b.store.id.0),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;
    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "A must not be able to export B's store's payments by naming its id directly"
    );

    // Positive control, same reasoning as the invoice export above.
    let own = server::api::invoices::export_payments_csv(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(a.store.id.0),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await
    .expect("A must be able to export A's own store's payments by naming its id directly");
    assert_eq!(own.status(), StatusCode::OK);
}

#[tokio::test]
#[ignore]
async fn export_payments_csv_with_a_nil_store_id_is_refused_like_any_foreign_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::invoices::export_payments_csv(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::invoices::ListPaymentsQuery {
            store_id: Some(Uuid::nil()),
            status: None,
            search: None,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(
        status_of(result),
        StatusCode::FORBIDDEN,
        "a nil store_id must not be treated as 'every store'"
    );
}

// ============================================================================
// Dashboard: aggregate counters and volume, scoped by `get_stores_for_user`
// rather than a client-supplied `store_id` - the leak to guard against here
// is another tenant's rows folding into the caller's own totals.
// ============================================================================

#[tokio::test]
#[ignore]
async fn get_stats_never_counts_another_tenants_data() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let stats =
        server::api::dashboard::get_stats(AuthenticatedUser(user_info(a.user_id)), State(state))
            .await
            .expect("a merchant must be able to read their own dashboard stats");

    assert_eq!(
        stats.total_stores, 1,
        "A's store count must not include B's store"
    );
    assert_eq!(
        stats.total_invoices, 1,
        "A's invoice count must not include B's invoice"
    );
}

#[tokio::test]
#[ignore]
async fn get_analytics_never_counts_another_tenants_data() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let _b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let analytics = server::api::dashboard::get_analytics(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Query(server::api::dashboard::AnalyticsQuery { days: None }),
    )
    .await
    .expect("a merchant must be able to read their own dashboard analytics");

    assert_eq!(
        analytics.total_payments, 1,
        "A's payment volume must not include B's payment"
    );
}

// ============================================================================
// Store members, webhook config and token policy: the same
// `require_store_settings_permission`/permission-gated `Path<Uuid>` shape as
// the store wallet endpoints above, tested separately because each is its
// own handler with its own copy of the guard.
// ============================================================================

#[tokio::test]
#[ignore]
async fn list_store_members_refuses_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    // None of the seeded default roles grant `canviewstoreusers`, including
    // Owner, so a caller needs a role built for it before a positive control
    // against A's own store means anything.
    grant_store_permission(&pg, &a, "ethpay.store.canviewstoreusers").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::list_store_members(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to list B's store's members"
    );

    // Positive control: without this, an endpoint that refuses every caller
    // would pass the assertion above for the wrong reason.
    let _ = server::api::stores::list_store_members(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to list A's own store's members");
}

#[tokio::test]
#[ignore]
async fn get_store_webhook_refuses_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    // Both tenants need a webhook actually configured, or the positive
    // control would fail with NOT_FOUND for a reason unrelated to tenancy.
    let _a_delivery = seed_webhook_delivery(&pg, &a).await;
    let _b_delivery = seed_webhook_delivery(&pg, &b).await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::get_store_webhook(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to read B's store's webhook configuration"
    );

    // Positive control, same reasoning as above.
    let own = server::api::stores::get_store_webhook(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to read A's own store's webhook configuration");
    assert_eq!(own.store_id, a.store.id.0);
}

#[tokio::test]
#[ignore]
async fn get_token_policy_refuses_a_non_members_store() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let b = seed_tenant(&pg, "b").await;
    let state = app_state(Arc::new(pg));

    let result = server::api::stores::get_token_policy(
        AuthenticatedUser(user_info(a.user_id)),
        State(state.clone()),
        Path(b.store.id.0),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        StatusCode::FORBIDDEN,
        "A must not be able to read B's store's token policy"
    );

    // Positive control: without this, an endpoint that refuses every caller
    // would pass the assertion above for the wrong reason. No policy is
    // configured, so success here is `Ok(None)`, not an error.
    let _ = server::api::stores::get_token_policy(
        AuthenticatedUser(user_info(a.user_id)),
        State(state),
        Path(a.store.id.0),
    )
    .await
    .expect("A must be able to read A's own store's token policy");
}

// ============================================================================
// Plugin pages: viewer and account come from the session, never the request
// ============================================================================

/// A `render_page` implementation that records every request it is handed,
/// so the test can check what the host told the plugin rather than trusting
/// a doc comment.
struct RecordingRenderer(Arc<Mutex<Vec<PageRequest>>>);

#[async_trait]
impl PageRenderer for RecordingRenderer {
    async fn render_page(
        &self,
        request: &PageRequest,
    ) -> Result<Option<PageElement>, PageRenderError> {
        self.0.lock().unwrap().push(request.clone());
        Ok(None)
    }
}

/// The billing plugin's merchant/admin split depends on `render_page` being
/// told the truth about who is asking. `get_page` resolves `viewer` and
/// `account_id` only from the authenticated `UserInfo` it is handed - never
/// from the path or query - so two different callers must never be recorded
/// as the same account, each must see their own identity, not the other
/// one's, and a `ServerAdmin` caller must be recorded as `Viewer::Admin`,
/// not `Viewer::Merchant`.
///
/// No real Postgres needed: `resolve_plugin`'s lookup fails against the
/// lazily-connecting pool and falls back to treating the path segment as a
/// literal plugin id, exactly as `server/src/api/plugins.rs`'s own
/// `a_registered_renderer_is_reachable_over_http` test relies on. The pool
/// points at the reserved `.invalid` TLD (RFC 2606) rather than `localhost`,
/// so the lookup fails on DNS resolution alone - no environment can make it
/// succeed by happening to have a database of that name reachable locally,
/// which would silently swap this test onto a different code path than the
/// one it means to cover.
#[tokio::test]
async fn plugin_page_viewer_and_account_are_always_the_callers_own() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let plugin_id = PluginId::new("cash.random.billing").unwrap();

    let mut pages = PageHost::new();
    pages.register(plugin_id, Arc::new(RecordingRenderer(seen.clone())));

    let pool = sqlx::PgPool::connect_lazy("postgres://nonexistent.invalid/nonexistent").unwrap();
    let mut state = app_state(Arc::new(PgDataService::new(pool)));
    state.plugin_pages = Arc::new(pages);

    let account_a = Uuid::new_v4();
    let account_b = Uuid::new_v4();
    let account_admin = Uuid::new_v4();

    let _ = server::api::plugins::get_page(
        State(state.clone()),
        AuthenticatedUser(user_info(account_a)),
        Path((
            "cash.random.billing".to_string(),
            "subscriptions".to_string(),
        )),
    )
    .await;
    let _ = server::api::plugins::get_page(
        State(state.clone()),
        AuthenticatedUser(user_info(account_b)),
        Path((
            "cash.random.billing".to_string(),
            "subscriptions".to_string(),
        )),
    )
    .await;
    let _ = server::api::plugins::get_page(
        State(state),
        AuthenticatedUser(user_info_with_role(account_admin, auth::Role::ServerAdmin)),
        Path((
            "cash.random.billing".to_string(),
            "subscriptions".to_string(),
        )),
    )
    .await;

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        3,
        "all three requests must have reached the renderer"
    );
    assert_eq!(seen[0].viewer, Viewer::Merchant);
    assert_eq!(
        seen[0].account_id.as_deref(),
        Some(account_a.to_string()).as_deref()
    );
    assert_eq!(seen[1].viewer, Viewer::Merchant);
    assert_eq!(
        seen[1].account_id.as_deref(),
        Some(account_b.to_string()).as_deref()
    );
    assert_ne!(
        seen[0].account_id, seen[1].account_id,
        "two different callers must never be recorded under the same account"
    );
    assert_eq!(
        seen[2].viewer,
        Viewer::Admin,
        "a ServerAdmin caller must be recorded as the admin viewer, not the merchant one"
    );
}
