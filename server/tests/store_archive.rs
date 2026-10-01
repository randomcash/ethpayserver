#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Archiving a store hides it and stops new invoices, and nothing else.
//!
//! Archiving is the routine way to get a store out of sight, so it must never
//! strand money already owed: an invoice created before the archive keeps its
//! watch, is credited when paid, and settles. These tests drive the real
//! router with a real `Authorization: Bearer` key, so the routes, the
//! extractors and the handlers are all on the path, not just the handler
//! bodies.
//!
//! Needs `DATABASE_URL`; skips when unset, like the other ignored integration
//! tests, and runs in CI's `--run-ignored` step.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

use auth::{AuthConfig, AuthService, Store, UserId};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::{MonitorEvent, PaymentConfirmed, PaymentDetected};
use evm::monitor::{ChainHealth, EventBridge};
use evm::{Address, B256, U256};
use rates::NoOpRateProvider;
use server::EventConsumer;
use server::services::RedisEVMMonitor;
use server::services::evm_monitor::{EVMMonitor, EVMMonitorError};
use server::state::PgAppState;
use types::{
    ChainId, InvoiceData, InvoiceId, InvoiceReader, InvoiceStatus, InvoiceWriter, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, StoreId, StorePaymentMethodWriter, WatchedAddressReader,
    WatchedAddressWriter,
};

/// A valid xpub (BIP-32 test vector 1, chain m/0H), used by no other test
/// file: one xpub can be registered to one account only, and the files share
/// a database.
const XPUB: &str = "xpub68Gmy5EdvgibQVfPdqkBBCHxA5htiqg55crXYuXoQRKfDBFA1WEjWgP6LHhwBZeNK1VTsfTFUHCdrfp1bgwQ9xv5ski8PX9rL2dZXvgGDnw";

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");
    Some(PgDataService::new(pool))
}

struct Merchant {
    store: Store,
    key: String,
}

/// The one account allowed to hold `XPUB`, so a rerun against the same
/// database registers it to the same account instead of a new one.
const WALLET_OWNER: Uuid = Uuid::from_u128(0x5707_e0a2_c41e_4a11_9c3d_0000_0000_0001);

/// A user, a store they own, and an API key. `with_wallet` adds an ETH payment
/// method so invoices can be created; it also pins the user to `WALLET_OWNER`.
async fn seed_merchant(pg: &PgDataService, with_wallet: bool) -> Merchant {
    let user_id = if with_wallet {
        WALLET_OWNER
    } else {
        Uuid::new_v4()
    };
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, \
             '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"\"}'::jsonb, \
             '{\"ciphertext\":\"\",\"iv\":\"\",\"mac\":\"\"}'::jsonb, \
             'h', 'passkey:' || $1::text) ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .execute(pg.pool())
    .await
    .expect("seed user");

    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(user_id));
    pg.create_store_owned_by(&store, UserId(user_id))
        .await
        .expect("seed store");
    if with_wallet {
        StorePaymentMethodWriter::create_payment_method(
            pg,
            store.id.0,
            &ChainId::evm(1),
            None,
            "ETH",
            18,
            Some(XPUB),
        )
        .await
        .expect("seed payment method");
    }

    let key = format!("ak_test_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix) \
         VALUES ($1, $2, 'archive test key', $3, 'ak_test')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(hex::encode(Sha256::digest(key.as_bytes())))
    .execute(pg.pool())
    .await
    .expect("seed api key");

    Merchant { store, key }
}

/// The router `main` serves, over the production auth-service type.
fn app(pg: &Arc<PgDataService>) -> Router {
    let auth_service = Arc::new(AuthService::with_config(
        Arc::clone(pg),
        AuthConfig::default(),
    ));
    let state = PgAppState::new(
        Arc::clone(pg),
        auth_service,
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    );
    server::api::router(state, false, None, None, None)
}

async fn call(
    app: &Router,
    key: &str,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {key}"));
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).expect("build request"))
        .await
        .expect("router call");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn store_ids(app: &Router, key: &str, uri: &str) -> Vec<String> {
    let (status, body) = call(app, key, Method::GET, uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body.as_array()
        .expect("store list is an array")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_string())
        .collect()
}

fn invoice_body(store_id: Uuid) -> serde_json::Value {
    serde_json::json!({ "store_id": store_id, "currency": "ETH", "amount": "1.00" })
}

#[tokio::test]
#[ignore]
async fn list_hides_archived_and_unarchive_brings_the_store_back() {
    let Some(pg) = service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let m = seed_merchant(&pg, false).await;
    let app = app(&pg);
    let id = m.store.id.0.to_string();

    // Positive control: a live store is listed, so the absences below mean
    // "filtered", not "the list is empty for some other reason".
    assert!(store_ids(&app, &m.key, "/stores").await.contains(&id));

    let uri = format!("/stores/{id}");
    let (status, _) = call(&app, &m.key, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Archiving an archived store is a no-op success.
    let (status, _) = call(&app, &m.key, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(!store_ids(&app, &m.key, "/stores").await.contains(&id));
    let archived = store_ids(&app, &m.key, "/stores?include_archived=true").await;
    assert!(archived.contains(&id));

    let unarchive = format!("/stores/{id}/unarchive");
    let (status, _) = call(&app, &m.key, Method::POST, &unarchive, None).await;
    assert_eq!(status, StatusCode::OK);
    // Idempotent.
    let (status, _) = call(&app, &m.key, Method::POST, &unarchive, None).await;
    assert_eq!(status, StatusCode::OK);

    assert!(store_ids(&app, &m.key, "/stores").await.contains(&id));
}

#[tokio::test]
#[ignore]
async fn only_the_owner_can_unarchive() {
    let Some(pg) = service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let owner = seed_merchant(&pg, false).await;
    let stranger = seed_merchant(&pg, false).await;
    let app = app(&pg);
    let uri = format!("/stores/{}/unarchive", owner.store.id.0);

    let (status, _) = call(&app, &stranger.key, Method::POST, &uri, None).await;
    assert_ne!(
        status,
        StatusCode::NO_CONTENT,
        "a stranger must not unarchive"
    );
    assert!(status.is_client_error());
}

#[tokio::test]
#[ignore]
async fn invoice_creation_is_refused_on_an_archived_store_and_works_when_unarchived() {
    let Some(pg) = service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let m = seed_merchant(&pg, true).await;
    let app = app(&pg);
    let id = m.store.id.0;

    // Positive control: this exact request succeeds on the live store, so the
    // refusal below is the archive and not a bad request.
    let (status, body) = call(
        &app,
        &m.key,
        Method::POST,
        "/invoices",
        Some(invoice_body(id)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, _) = call(&app, &m.key, Method::DELETE, &format!("/stores/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(
        &app,
        &m.key,
        Method::POST,
        "/invoices",
        Some(invoice_body(id)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "store_archived");

    let (status, _) = call(
        &app,
        &m.key,
        Method::POST,
        &format!("/stores/{id}/unarchive"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &app,
        &m.key,
        Method::POST,
        "/invoices",
        Some(invoice_body(id)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// Stands in for the monitor, which the consumer only needs as a type.
struct NoopEVMMonitor;

#[async_trait]
impl EVMMonitor for NoopEVMMonitor {
    async fn watch_address(
        &self,
        _: &ChainId,
        _: Address,
        _: Uuid,
        _: Option<U256>,
        _: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }
    async fn watch_address_by_chain_id(
        &self,
        _: u64,
        _: Address,
        _: Uuid,
        _: Option<U256>,
        _: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }
    async fn unwatch_address(
        &self,
        _: &ChainId,
        _: Address,
        _: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }
    async fn unwatch_address_by_chain_id(
        &self,
        _: u64,
        _: Address,
        _: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }
    async fn health_check(&self) -> Result<(), EVMMonitorError> {
        Ok(())
    }
    async fn get_chain_health(&self) -> Result<Vec<ChainHealth>, EVMMonitorError> {
        Ok(vec![])
    }
}

/// A pending $100 invoice on `store_id` with one ETH option and an active
/// watch on its address.
async fn seed_watched_invoice(
    pg: &PgDataService,
    store_id: Uuid,
    chain: &ChainId,
) -> (InvoiceData, PaymentOptionData, Address) {
    let invoice = InvoiceData {
        id: InvoiceId::new(),
        store_id: StoreId(store_id),
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
    InvoiceWriter::upsert(pg, &invoice).await.unwrap();

    let payment_address = Address::random();
    let address_str = format!("{payment_address:#x}");
    let option = PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice.id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", chain),
        chain_id: chain.clone(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: address_str.clone(),
        wallet_id: None,
        derivation_index: None,
        amount: "50000000000000000".to_string(),
        rate: Some("0.0005".to_string()),
        rate_at: Some(Utc::now()),
        is_active: true,
        created_at: Utc::now(),
    };
    data_service::PaymentOptionWriter::create(pg, &option)
        .await
        .unwrap();
    WatchedAddressWriter::upsert(pg, &address_str, &option.id, chain, None)
        .await
        .unwrap();

    (invoice, option, payment_address)
}

/// Publish a detection and then a confirmation of a payment covering the
/// whole invoice, as the monitor would.
async fn deliver_full_payment(
    bridge: &MemoryBridge,
    eip155: u64,
    invoice: &InvoiceData,
    payment_address: Address,
) {
    let tx_hash = B256::random();
    let amount = U256::from(50_000_000_000_000_000u64);
    let invoice_uuid = Uuid::parse_str(invoice.id.as_str()).unwrap();
    bridge
        .publish(&MonitorEvent::PaymentDetected(PaymentDetected {
            chain_id: eip155,
            invoice_id: invoice_uuid,
            payment_address,
            amount,
            tx_hash,
            block_number: 100,
            block_hash: B256::random(),
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::random(),
            confirmations: 1,
            required_confirmations: 3,
            detected_at: Utc::now(),
        }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    bridge
        .publish(&MonitorEvent::PaymentConfirmed(PaymentConfirmed {
            tx_index: -1,
            chain_id: eip155,
            invoice_id: invoice_uuid,
            payment_address,
            amount,
            tx_hash,
            block_number: 100,
            confirmations: 3,
            confirmed_at: Utc::now(),
        }))
        .await
        .unwrap();
}

/// Archive a store that has a pending invoice with an active watch, then pay
/// it. The watch must survive the archive and the invoice must settle.
///
/// The watch is asserted directly as well as through settlement: a payment to
/// an expired watch is still credited by a fallback, so settlement alone would
/// not notice an archive that quietly stopped watching.
#[tokio::test]
#[ignore]
async fn an_invoice_on_an_archived_store_is_still_watched_and_settles() {
    let Some(pg) = service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let m = seed_merchant(&pg, false).await;
    let app = app(&pg);

    // A chain id no other run has used: the consumer keeps a durable per-chain
    // cursor, and a rerun against the same database with a fresh in-memory
    // bridge would otherwise resume against the previous run's cursor.
    let eip155 = 1_000_000 + u64::from(Uuid::new_v4().as_u128() as u32);
    let chain = ChainId::evm(eip155);
    let (invoice, option, payment_address) = seed_watched_invoice(&pg, m.store.id.0, &chain).await;
    let address_str = format!("{payment_address:#x}");

    let (status, _) = call(
        &app,
        &m.key,
        Method::DELETE,
        &format!("/stores/{}", m.store.id.0),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let watched = WatchedAddressReader::get_payment_option_id(&*pg, &address_str, &chain, None)
        .await
        .unwrap();
    assert_eq!(
        watched,
        Some(option.id.clone()),
        "archiving the store must leave the invoice's watch active"
    );

    let bridge = Arc::new(MemoryBridge::new());
    let consumer: EventConsumer<PgDataService, NoopEVMMonitor> = EventConsumer::new(
        Arc::clone(&bridge) as Arc<dyn EventBridge>,
        Arc::clone(&pg),
        None,
        None,
        None,
        Arc::new(server::services::email::NoopEmailSender),
    )
    // The consumer resumes from every chain's stored cursor, and this database
    // is shared with other tests whose cursors name a different bridge epoch.
    // Without accepting the break it refuses to start at all.
    .with_accepted_lineage_break(true)
    // Fail the test with the reason; the default is `process::exit(1)`, which
    // kills the test binary without a message.
    .with_apply_failure_hook(Arc::new(|chain, seq| {
        panic!("consumer halted applying chain {chain} seq {seq}")
    }))
    .with_resume_failure_hook(Arc::new(|reason| panic!("consumer cannot continue: {reason}")));
    let consumer_handle = tokio::spawn(async move { consumer.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    deliver_full_payment(&bridge, eip155, &invoice, payment_address).await;

    let mut status = InvoiceStatus::Pending;
    for _ in 0..50 {
        status = InvoiceReader::get(&*pg, &invoice.id)
            .await
            .unwrap()
            .expect("invoice exists")
            .status;
        if status == InvoiceStatus::Paid {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    consumer_handle.abort();
    assert_eq!(
        status,
        InvoiceStatus::Paid,
        "a payment to an invoice on an archived store must still settle it"
    );
}
