#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Review finding, fixed: `BulkMerchantVolumeReader::merchant_volumes` shipped
//! with no test exercising the composition it adds over the single-account
//! reader - the store-to-account attribution (`store_owner`) and the
//! cross-account rate batching. A bug in either could blend one account's
//! volume into another's, which is exactly the property an operator table
//! keyed on "who is about to cross a bracket" cannot afford to get wrong.
//!
//! This seeds two accounts, each owning one store with payments in a
//! different asset, and asserts the bulk answer for both together matches
//! what [`MerchantVolumeReader::merchant_volume`] returns for each alone -
//! proving the batch neither drops nor bleeds volume across accounts.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use auth::{Result as AuthResult, Session, SessionId, SessionService, Store, UserId, UserInfo};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::{ExchangeRate, RateError, RateProvider};
use server::services::RedisEVMMonitor;
use server::services::plugins::{
    BulkMerchantVolumeReader, MerchantVolumeReader, PluginMerchantVolume,
};
use server::state::PgAppState;
use types::{
    AssetType, ChainId, InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentData,
    PaymentWriter, StoreId,
};

/// Not exercised: `merchant_volume`/`merchant_volumes` never touch session
/// management, only the data service and the rate provider.
struct UnusedSessionService;

#[async_trait]
impl SessionService for UnusedSessionService {
    async fn validate_session(&self, _session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        unimplemented!("not exercised by merchant volume reads")
    }
    async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by merchant volume reads")
    }
    async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised by merchant volume reads")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised by merchant volume reads")
    }
}

/// A rate provider with two fixed, direct pairs and nothing else - enough to
/// price the two assets this test seeds without a network call.
struct FixedRateProvider {
    rates: Vec<(&'static str, &'static str, Decimal)>,
}

#[async_trait]
impl RateProvider for FixedRateProvider {
    async fn get_rate(&self, from: &str, to: &str) -> Result<ExchangeRate, RateError> {
        self.rates
            .iter()
            .find(|(f, t, _)| *f == from && *t == to)
            .map(|(_, _, rate)| ExchangeRate {
                from: from.to_string(),
                to: to.to_string(),
                rate: *rate,
                timestamp: Utc::now(),
            })
            .ok_or_else(|| RateError::UnsupportedPair {
                from: from.to_string(),
                to: to.to_string(),
            })
    }

    fn name(&self) -> &'static str {
        "fixed"
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

/// Seed a store owned by `owner`, with one payment of `amount` of `asset`
/// (18 decimals, the no-`payment_option` default) landing today.
async fn seed_store_with_payment(
    pg: &PgDataService,
    owner: Uuid,
    asset: &str,
    amount: &str,
) -> StoreId {
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(owner));
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store owned by user");

    let invoice = InvoiceData {
        id: InvoiceId::new(),
        store_id: store.id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: amount.to_string(),
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
        invoice_id: invoice.id,
        payment_option_id: None,
        chain_id: ChainId::evm(1),
        asset_type: AssetType::Native,
        amount: amount.to_string(),
        asset_symbol: asset.to_string(),
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

    store.id
}

fn state(data_service: Arc<PgDataService>) -> PgAppState<UnusedSessionService> {
    PgAppState::new(
        data_service,
        Arc::new(UnusedSessionService),
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(FixedRateProvider {
            // 1 USD buys 0.0004 ETH (2500 USD/ETH), and DAI holds its peg.
            rates: vec![
                ("USD", "ETH", Decimal::from_str_exact("0.0004").unwrap()),
                ("USD", "DAI", Decimal::ONE),
            ],
        }),
        Arc::new(server::services::email::NoopEmailSender),
    )
}

/// The property the review flagged as untested: a batched read across two
/// accounts must return exactly what looping the single-account reader would
/// have, neither dropping a store's volume nor blending it into the other
/// account's total.
#[tokio::test]
#[ignore]
async fn bulk_volume_matches_the_single_account_reader_and_does_not_blend_accounts() {
    let Some(pg) = service().await else {
        return;
    };
    let account_a = seed_user(pg.pool()).await;
    let account_b = seed_user(pg.pool()).await;

    // Account A: 2 ETH at $2500/ETH = $5000.
    seed_store_with_payment(&pg, account_a, "ETH", "2000000000000000000").await;
    // Account B: 100 DAI at parity = $100.
    seed_store_with_payment(&pg, account_b, "DAI", "100000000000000000000").await;

    let reader = PluginMerchantVolume::new(state(Arc::new(pg)));

    let single_a = reader
        .merchant_volume(UserId(account_a), 30, "USD")
        .await
        .expect("single-account read for A");
    let single_b = reader
        .merchant_volume(UserId(account_b), 30, "USD")
        .await
        .expect("single-account read for B");
    assert_eq!(single_a.volume, "5000");
    assert_eq!(single_b.volume, "100");

    let bulk = reader
        .merchant_volumes(&[UserId(account_a), UserId(account_b)], 30, "USD")
        .await
        .expect("bulk read");

    assert_eq!(bulk.len(), 2, "one entry per account, not merged into one");
    let by_a = bulk
        .iter()
        .find(|v| v.account_id == UserId(account_a))
        .expect("account A present in the batch");
    let by_b = bulk
        .iter()
        .find(|v| v.account_id == UserId(account_b))
        .expect("account B present in the batch");

    assert_eq!(
        by_a.volume, single_a,
        "the batched answer for A must match what looping the single-account reader gives"
    );
    assert_eq!(
        by_b.volume, single_b,
        "the batched answer for B must match what looping the single-account reader gives"
    );
    assert_eq!(
        by_a.volume.volume, "5000",
        "A's ETH volume must not include B's DAI"
    );
    assert_eq!(
        by_b.volume.volume, "100",
        "B's DAI volume must not include A's ETH"
    );
}

/// An account with no stores must still come back with a zero entry rather
/// than being silently dropped from the batch - the same behaviour the
/// module doc comment promises for the empty-store case.
#[tokio::test]
#[ignore]
async fn an_account_with_no_stores_gets_a_zero_entry_not_a_dropped_one() {
    let Some(pg) = service().await else {
        return;
    };
    let has_stores = seed_user(pg.pool()).await;
    let no_stores = seed_user(pg.pool()).await;
    seed_store_with_payment(&pg, has_stores, "ETH", "2000000000000000000").await;

    let reader = PluginMerchantVolume::new(state(Arc::new(pg)));
    let bulk = reader
        .merchant_volumes(&[UserId(has_stores), UserId(no_stores)], 30, "USD")
        .await
        .expect("bulk read");

    assert_eq!(
        bulk.len(),
        2,
        "the storeless account must still get an entry"
    );
    let empty = bulk
        .iter()
        .find(|v| v.account_id == UserId(no_stores))
        .expect("storeless account present in the batch");
    assert_eq!(empty.volume.volume, "0");
}
