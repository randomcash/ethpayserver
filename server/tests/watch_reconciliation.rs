#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Proves the watch reconciler can actually go red, in both directions, on
//! purpose.
//!
//! Neither fault has an easy accidental trigger: `watched_addresses` cascades
//! away with its invoice, so Postgres alone can never hold an "orphaned" row,
//! and a healthy deploy never leaves Redis and Postgres disagreeing either.
//! Seeding one of each here is the only way to show this check can fail
//! rather than being vacuously green - the same standard this repo holds
//! every check like it to.
//!
//! Needs a real Postgres and a real Redis; skips (not fails) when either
//! `DATABASE_URL` or `TEST_REDIS_URL` is unset, the convention every other
//! ignored integration test in this crate follows.

use auth::{Store, UserId};
use data_service::invoice_creation::InvoiceCreationWriter;
use data_service::store_creation::StoreCreationWriter;
use data_service::{
    ExpectedWatch, LiveWatchedAddressReader, LiveWatchedAddressWriter, PgDataService,
    RedisDataService, WatchKey, reconcile,
};
use server::services::{RedisEVMMonitor, reconcile_watches};
use sqlx::PgPool;
use types::{
    ChainId, InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentMethodId,
    PaymentOptionData, PaymentOptionId, StoreId,
};
use uuid::Uuid;

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .unwrap_or_else(|e| panic!("DATABASE_URL is set but connecting failed: {e}"));
    Some(PgDataService::new(pool))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier, role) \
         VALUES ($1, \
           '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"\"}'::jsonb, \
           '{\"ciphertext\":\"\",\"iv\":\"\",\"mac\":\"\"}'::jsonb, \
           'h', 'passkey:' || $1::text, 'user')",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_pending_invoice(pool: &PgPool, store: Uuid) -> String {
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

/// An invoice and its single native-asset option, shaped for
/// `InvoiceCreationWriter::create_invoice_with_options` - the real entry
/// point a new invoice is created through, as opposed to `seed_pending_invoice`
/// and `seed_payment_option` below, which write rows directly and skip it.
fn an_invoice_and_option(store: Uuid, address: &str) -> (InvoiceData, PaymentOptionData) {
    let chain = ChainId::evm(11155111);
    let invoice = InvoiceData {
        id: InvoiceId(format!("inv-{}", Uuid::new_v4())),
        store_id: StoreId(store),
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: "10".to_string(),
        amount_received: "0".to_string(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    let option = PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice.id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", &chain),
        chain_id: chain,
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: address.to_string(),
        wallet_id: None,
        derivation_index: None,
        amount: "1".to_string(),
        rate: None,
        rate_at: None,
        is_active: true,
        created_at: chrono::Utc::now(),
    };
    (invoice, option)
}

async fn seed_payment_option(pool: &PgPool, invoice: &str, address: &str) -> Uuid {
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

async fn seed_watched_address(pool: &PgPool, invoice: &str, payment_option: Uuid, address: &str) {
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

async fn cleanup(pool: &PgPool, users: &[Uuid]) {
    for user in users {
        let _ = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(pool)
            .await;
    }
}

/// Seeds a fault in each direction on purpose:
///
/// - a Redis key with no invoice behind it in Postgres at all, standing in
///   for what a deleted or resolved invoice leaves behind - **stale**.
/// - a live, pending invoice's watched address on record in Postgres,
///   deliberately never told to Redis - **missed**, the worse fault, since a
///   real payment to it would go uncredited.
///
/// A prior pass of this reconciler that only checked one direction would
/// have passed a suite covering only the other, so both are asserted here in
/// the same test rather than split across two.
#[tokio::test]
#[ignore]
async fn reconcile_watches_reports_a_stale_watch_and_a_missed_watch() {
    let Some(pg) = service().await else {
        return;
    };
    let Some(redis_url) = std::env::var("TEST_REDIS_URL").ok() else {
        return;
    };

    let monitor = RedisEVMMonitor::connect(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("TEST_REDIS_URL is set but connecting failed: {e}"));
    let live_watches = RedisDataService::new(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("TEST_REDIS_URL is set but connecting failed: {e}"));

    let owner = seed_user(pg.pool()).await;
    let store = Store::new(
        format!("watch-reconciler-test-{}", Uuid::new_v4()),
        UserId(owner),
    );
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store");

    // The missed-watch case.
    let invoice = seed_pending_invoice(pg.pool(), store.id.0).await;
    let missed_address = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let payment_option = seed_payment_option(pg.pool(), &invoice, &missed_address).await;
    seed_watched_address(pg.pool(), &invoice, payment_option, &missed_address).await;

    // The stale-watch case.
    let stale_address = format!("0x{:040x}", Uuid::new_v4().as_u128());
    live_watches
        .watch_address(
            &stale_address,
            &InvoiceId::from_string("a-deleted-invoice".to_string()),
            &ChainId::evm(11155111),
            None,
        )
        .await
        .expect("seed a stale redis watch");

    let expected: Vec<WatchKey> = pg
        .get_expected_watched_addresses()
        .await
        .expect("read the expected set")
        .iter()
        .map(ExpectedWatch::key)
        .collect();
    let actual: Vec<WatchKey> = live_watches
        .get_all_watched()
        .await
        .expect("read the actual set")
        .into_iter()
        .map(|(address, _invoice_id, chain_id, token)| {
            WatchKey::new(chain_id, &address, token.as_deref())
        })
        .collect();

    let diff = reconcile(&expected, &actual);
    let missed_key = WatchKey::new(ChainId::evm(11155111), &missed_address, None);
    let stale_key = WatchKey::new(ChainId::evm(11155111), &stale_address, None);
    assert!(
        diff.missed.contains(&missed_key),
        "a live invoice's never-watched address must be reported missed"
    );
    assert!(
        diff.stale.contains(&stale_key),
        "a redis-only key with no invoice behind it must be reported stale"
    );

    // `reconcile_watches` is the exact function `/health/deep` calls - prove
    // it, not just the pieces it is built from, sees both.
    let counts = reconcile_watches(&pg, &monitor)
        .await
        .expect("reconcile via the real entry point");
    assert!(counts.stale >= 1);
    assert!(counts.missed >= 1);

    let _ = live_watches
        .unwatch_address(&stale_address, &ChainId::evm(11155111), None)
        .await;
    cleanup(pg.pool(), &[owner]).await;
}

/// `WatchKey` deliberately drops the invoice id - see its doc comment for
/// why that is safe rather than merely convenient. This is the "why" made
/// concrete: a stale Redis watch for a deleted invoice, then the same
/// address legitimately rewatched for a second invoice, must leave Redis
/// holding the *new* invoice id (proving the plain Redis `SET` in
/// `watch_address` really does overwrite in place) and the reconciler
/// reporting nothing for that key (proving the key-only comparison does not
/// need the invoice id to be safe here).
///
/// Invoice B is created through `create_invoice_with_options`, the same
/// entry point a real invoice goes through, rather than by hand-inserting a
/// `watched_addresses` row: that function's `ON CONFLICT (address, chain_id,
/// token_address) DO UPDATE` is what actually reassigns the row in
/// production, and it never touches the row's `invoice_id` column directly -
/// only `payment_option_id`, which the `expected_watched_addresses` view
/// joins through to get the invoice. Exercising anything less than the real
/// upsert would not prove the thing this test exists to prove.
///
/// Invoice A is resolved rather than deleted, for the same reason: deleting
/// its row would remove the very conflict this test needs invoice B's write
/// to hit, and would prove the easier, less realistic case instead.
#[tokio::test]
#[ignore]
#[allow(clippy::too_many_lines)] // integration test with multi-step setup + assertions
async fn a_reused_address_resolves_to_the_new_invoice_not_the_stale_one() {
    let Some(pg) = service().await else {
        return;
    };
    let Some(redis_url) = std::env::var("TEST_REDIS_URL").ok() else {
        return;
    };

    let monitor = RedisEVMMonitor::connect(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("TEST_REDIS_URL is set but connecting failed: {e}"));
    let live_watches = RedisDataService::new(&redis_url)
        .await
        .unwrap_or_else(|e| panic!("TEST_REDIS_URL is set but connecting failed: {e}"));

    let owner = seed_user(pg.pool()).await;
    let store = Store::new(
        format!("watch-reuse-test-{}", Uuid::new_v4()),
        UserId(owner),
    );
    pg.create_store_owned_by(&store, UserId(owner))
        .await
        .expect("seed store");

    let address = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let chain = ChainId::evm(11155111);

    // Invoice A watches the address, then resolves (paid) while its
    // `watched_addresses` row stays `is_active = TRUE` - the cleanup-lag
    // window the view's doc comment describes - and Redis is never told.
    // The row is not deleted: production reuse never deletes it either (see
    // below), so leaving it in place is what makes the second half of this
    // test exercise the real code path instead of an easier one.
    let invoice_a = seed_pending_invoice(pg.pool(), store.id.0).await;
    let payment_option_a = seed_payment_option(pg.pool(), &invoice_a, &address).await;
    seed_watched_address(pg.pool(), &invoice_a, payment_option_a, &address).await;
    live_watches
        .watch_address(
            &address,
            &InvoiceId::from_string(invoice_a.clone()),
            &chain,
            None,
        )
        .await
        .expect("seed invoice A's watch");
    InvoiceWriter::update_status(
        &pg,
        &InvoiceId::from_string(invoice_a.clone()),
        InvoiceStatus::Paid,
    )
    .await
    .expect("resolve invoice A without touching its watched_addresses row");

    let key = WatchKey::new(chain.clone(), &address, None);
    let stale_counts = reconcile_watches(&pg, &monitor)
        .await
        .expect("reconcile before reuse");
    assert!(
        stale_counts.stale >= 1,
        "invoice A's watch must be reported stale before the address is reused"
    );

    // Invoice B legitimately rewatches the same address through the real
    // write path (`InvoiceCreationWriter::create_invoice_with_options`),
    // not a hand-written INSERT. That function's `ON CONFLICT (address,
    // chain_id, token_address) DO UPDATE` fires here because invoice A's row
    // is still physically present, and its `SET` clause only ever touches
    // `payment_option_id` - never `invoice_id` - so the same underlying row
    // that used to belong to invoice A is repointed to invoice B's option
    // without the `invoice_id` column itself ever being written. Proving the
    // reconciler goes quiet after this is what proves dropping `invoice_id`
    // from `WatchKey` is safe against the actual conflict-update path, not
    // just a delete-then-insert that never exercises it.
    let (invoice_b, option_b) = an_invoice_and_option(store.id.0, &address);
    let invoice_b_id = invoice_b.id.clone();
    pg.create_invoice_with_options(&invoice_b, std::slice::from_ref(&option_b))
        .await
        .expect("legitimately rewatch the address for invoice B via the real write path");
    live_watches
        .watch_address(&address, &invoice_b_id, &chain, None)
        .await
        .expect("the app's post-write watch_address call for invoice B");

    let redis_value = live_watches
        .get_watched_invoice(&address, &chain, None)
        .await
        .expect("read back the overwritten watch")
        .expect("the address is still watched");
    assert_eq!(
        redis_value.as_str(),
        invoice_b_id.as_str(),
        "the Redis value must have moved to invoice B, not stayed on the stale invoice A"
    );

    let expected: Vec<WatchKey> = pg
        .get_expected_watched_addresses()
        .await
        .expect("read the expected set")
        .iter()
        .map(ExpectedWatch::key)
        .collect();
    let actual: Vec<WatchKey> = live_watches
        .get_all_watched()
        .await
        .expect("read the actual set")
        .into_iter()
        .map(|(address, _invoice_id, chain_id, token)| {
            WatchKey::new(chain_id, &address, token.as_deref())
        })
        .collect();
    let diff = reconcile(&expected, &actual);
    assert!(
        !diff.stale.contains(&key) && !diff.missed.contains(&key),
        "once invoice B legitimately owns the address, the reconciler must report neither fault for it"
    );

    let _ = live_watches.unwatch_address(&address, &chain, None).await;
    cleanup(pg.pool(), &[owner]).await;
}
