#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The runbook for rotating a store's wallet key says two things that
//! surprise people: a new invoice derives from the new key, and an
//! already-created (pending) invoice keeps resolving on the address it was
//! already quoted. Both are asserted here against the real handlers - a real
//! invoice's payment address, computed by the same derivation code a
//! customer's payment actually watches - rather than against the rotation
//! response body, which only proves the audit trail, not that anything
//! downstream changed.
//!
//! Calls `create_invoice` and `rotate_store_wallet` directly against a real
//! database, the same pattern `plugin_invoice_creation_filter.rs` uses:
//! `AuthenticatedUser` and `State` are plain data the extractors produce, so
//! nothing here depends on routing or middleware.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{
    Result as AuthResult, Role, Session, SessionId, SessionService, Store, UserId, UserInfo,
};
use axum::response::IntoResponse;
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use evm::XpubDeriver;
use rates::NoOpRateProvider;
use server::api::ApiErr;
use server::api::AuthenticatedUser;
use server::api::invoices::{CreateInvoiceRequest, create_invoice};
use server::api::stores::{RotateWalletRequest, RotateWalletResponse, rotate_store_wallet};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;
use types::{ChainId, InvoiceId, PaymentOptionReader, StorePaymentMethodWriter};

/// A well-known BIP-32 test vector xpub (test vector 1's master key) -
/// stable across the whole test suite, chosen only because it is valid,
/// not because of what it derives from.
const OLD_XPUB: &str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";

/// The standard BIP-39 test mnemonic's account key at `m/44'/60'/0'` -
/// a second, distinct valid xpub to rotate onto.
const NEW_XPUB: &str = "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt";

struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by create_invoice or rotate_store_wallet")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by create_invoice or rotate_store_wallet")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by create_invoice or rotate_store_wallet")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by create_invoice or rotate_store_wallet")
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
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

fn invoice_request(store_id: Uuid) -> CreateInvoiceRequest {
    CreateInvoiceRequest {
        store_id,
        // Same-asset invoice: currency equals the payment method's asset
        // symbol, so creation needs no rate provider - only whether an
        // address can be derived is under test here.
        currency: "ETH".to_string(),
        amount: "1.00".to_string(),
        expiration_seconds: None,
        metadata: None,
        customer_email: None,
        webhook_url: None,
        redirect_url: None,
    }
}

/// Ticket's Verify criteria 1 and 2, against the real handlers: a rotation
/// changes what a *new* invoice derives from, and does not touch the address
/// an already-pending invoice was already quoted.
#[tokio::test]
#[ignore]
async fn rotation_moves_new_invoices_but_not_a_pending_ones_address() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    StorePaymentMethodWriter::create_payment_method(
        &pg,
        store.id.0,
        &ChainId::evm(1),
        None,
        "ETH",
        18,
        Some(OLD_XPUB),
    )
    .await
    .expect("seed a payment method pinned to the old key");

    let ds = Arc::new(pg);

    // Invoice created before the rotation - the one that must NOT move.
    let (status, Json(pending_invoice)) = create_invoice(
        AuthenticatedUser(user_info(owner)),
        State(app_state(Arc::clone(&ds))),
        Json(invoice_request(store.id.0)),
    )
    .await
    .expect("create the pending invoice");
    assert_eq!(status, StatusCode::CREATED);
    let pending_address = pending_invoice.payment_options[0].payment_address.clone();

    let expected_old_address = XpubDeriver::from_xpub("eip155", OLD_XPUB)
        .unwrap()
        .derive_address(0)
        .unwrap();
    assert_eq!(
        pending_address, expected_old_address,
        "the pending invoice must derive from the old key at its first index"
    );

    let rotated = rotate_store_wallet(
        AuthenticatedUser(user_info(owner)),
        State(app_state(Arc::clone(&ds))),
        Path(store.id.0),
        Json(RotateWalletRequest {
            xpub: NEW_XPUB.to_string(),
            reason: Some("test rotation".to_string()),
            namespace: "eip155".to_string(),
        }),
    )
    .await
    .expect("rotate the store's wallet");
    assert_eq!(rotated.methods_rotated, 1);
    assert_rotation_audit_trail(&rotated, OLD_XPUB, 1);

    // A new invoice after rotation must derive from the new key.
    let (status, Json(new_invoice)) = create_invoice(
        AuthenticatedUser(user_info(owner)),
        State(app_state(Arc::clone(&ds))),
        Json(invoice_request(store.id.0)),
    )
    .await
    .expect("create an invoice after rotation");
    assert_eq!(status, StatusCode::CREATED);
    let new_address = new_invoice.payment_options[0].payment_address.clone();

    let expected_new_address = XpubDeriver::from_xpub("eip155", NEW_XPUB)
        .unwrap()
        .derive_address(0)
        .unwrap();
    assert_eq!(
        new_address, expected_new_address,
        "a new invoice must derive from the new key, not the old one"
    );
    assert_ne!(
        new_address, pending_address,
        "the new invoice must not reuse the address already quoted to the pending one"
    );

    // The behaviour that surprises people: the pending invoice's own address
    // is exactly what it would go red on if rotation quietly re-derived it.
    let pending_id = InvoiceId(pending_invoice.id.clone());
    let refetched = PaymentOptionReader::get_for_invoice(&*ds, &pending_id)
        .await
        .expect("re-fetch the pending invoice's payment option");
    assert_eq!(
        refetched[0].payment_address, expected_old_address,
        "an existing pending invoice must keep resolving on its old-xpub address after rotation"
    );
}

/// The audit trail is the point of the rotation response: a rotation that
/// reports success but names the wrong previous key or index is worse than
/// one that fails outright. Checked against the real key just rotated away
/// from, not against a value the handler could echo back unexamined.
fn assert_rotation_audit_trail(rotated: &RotateWalletResponse, old_xpub: &str, old_index: i32) {
    assert_eq!(rotated.rotations.len(), 1);
    assert_eq!(
        rotated.rotations[0].previous_xpub_masked,
        api_types::mask_xpub(old_xpub)
    );
    assert_eq!(rotated.rotations[0].previous_derivation_index, old_index);
}

async fn error_status_and_message(err: ApiErr) -> (StatusCode, String) {
    let response = err.into_response();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read error body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("utf8 error body"),
    )
}

/// Ticket's Verify criterion 3: a malformed xpub is refused with a message
/// that says so, not the bare 400 a UI can only ever show as a generic
/// failure.
#[tokio::test]
#[ignore]
async fn rotation_refuses_a_malformed_xpub_with_a_specific_message() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    let ds = Arc::new(pg);

    let err = rotate_store_wallet(
        AuthenticatedUser(user_info(owner)),
        State(app_state(Arc::clone(&ds))),
        Path(store.id.0),
        Json(RotateWalletRequest {
            xpub: "not-a-real-extended-key".to_string(),
            reason: None,
            namespace: "eip155".to_string(),
        }),
    )
    .await
    .expect_err("a malformed xpub must be refused");

    let (status, body) = error_status_and_message(err).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("extended public key"),
        "message should say what was wrong, not stay a bare 400: {body}"
    );
}

/// Same criterion, the other named case: an `xprv` is refused on the same
/// version byte as any other malformed key, but the message has to say it
/// looks like a private key - the mistake someone rotating after a
/// compromise is most likely to make, and the one this product must never
/// silently accept.
#[tokio::test]
#[ignore]
async fn rotation_refuses_an_xprv_with_a_message_naming_it_a_private_key() {
    let Some(pg) = service().await else {
        return;
    };
    let owner = seed_user(pg.pool()).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");
    let ds = Arc::new(pg);

    // BIP-32 test vector 1's master private key - a real xprv, same key pair
    // OLD_XPUB above is the public half of.
    const XPRV: &str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";

    let err = rotate_store_wallet(
        AuthenticatedUser(user_info(owner)),
        State(app_state(Arc::clone(&ds))),
        Path(store.id.0),
        Json(RotateWalletRequest {
            xpub: XPRV.to_string(),
            reason: None,
            namespace: "eip155".to_string(),
        }),
    )
    .await
    .expect_err("an xprv must never be accepted as an xpub");

    let (status, body) = error_status_and_message(err).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("private key"),
        "message should distinguish a pasted private key from a mere typo: {body}"
    );
}
