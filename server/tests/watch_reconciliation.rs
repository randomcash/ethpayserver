#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Reconciling the watches the database expects against the watches the
//! monitor actually holds, against a real database and a real live-watch store.
//!
//! This is the test the feature exists for, so it is worth being explicit about
//! what it does and does not prove. It seeds a real discrepancy in each
//! direction and asserts each is reported as itself:
//!
//! - a live watch nothing in the database expects - wasteful only;
//! - an expected watch that has been removed from the live set - **nobody is
//!   watching an address an invoice expects payment on**, and a payment there
//!   arrives uncredited.
//!
//! The removed one is a *token* watch on an address whose *native* watch is
//! still held. That is deliberate: it is the case a comparison on the address
//! alone reports as healthy, and it is the case that loses a payment. If the
//! comparison key is ever narrowed to the address, this test is what goes red.
//!
//! It asserts membership rather than totals. The database and the live store
//! are shared with every other integration test, so unrelated entries are
//! expected in both directions and a count assertion here would fail for
//! reasons that have nothing to do with the feature.

use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use data_service::postgres::{WatchKey, reconcile_watches};
use data_service::{LiveWatchedAddressWriter, PgDataService, RedisDataService};
use types::{ChainId, InvoiceId};

/// Sepolia, the chain the rest of this suite seeds against.
const CHAIN_TEXT: &str = "eip155:11155111";

fn chain() -> ChainId {
    ChainId::evm(11155111)
}

/// A `DATABASE_URL` that is set but unreachable is not the same thing as one
/// that is not configured, and a test that quietly reports success in the
/// second case reports success in the first too.
async fn pg_service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .unwrap_or_else(|e| panic!("DATABASE_URL is set but connecting failed: {e}"));
    Some(PgDataService::new(pool))
}

async fn live_watches() -> Option<RedisDataService> {
    let url = std::env::var("TEST_REDIS_URL").ok()?;
    Some(
        RedisDataService::new(&url)
            .await
            .unwrap_or_else(|e| panic!("TEST_REDIS_URL is set but connecting failed: {e}")),
    )
}

fn unique_address() -> String {
    format!("0x{:040x}", Uuid::new_v4().as_u128())
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

async fn seed_store(pool: &PgPool, owner: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(format!("watch-reconciliation-{id}"))
        .bind(owner)
        .execute(pool)
        .await
        .expect("seed store");
    id
}

/// A pending, unexpired invoice: the case where a lost watch costs a payment.
async fn seed_invoice(pool: &PgPool, store: Uuid) -> String {
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

/// One payment option and its active watch row, for the native asset when
/// `token` is `None` and for that token contract otherwise.
///
/// Two of these on one address is the shape the whole comparison key exists
/// for: the same address, watched separately per asset.
async fn seed_watch(pool: &PgPool, invoice: &str, address: &str, token: Option<&str>) {
    let option_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO payment_options \
         (id, invoice_id, payment_method_id, chain_id, asset_type, asset_symbol, \
          token_address, payment_address, amount) \
         VALUES ($1, $2, $3, $4, $5::asset_type, $6, $7, $8, 1)",
    )
    .bind(option_id)
    .bind(invoice)
    .bind(if token.is_some() {
        "USDC-11155111"
    } else {
        "ETH-11155111"
    })
    .bind(CHAIN_TEXT)
    .bind(if token.is_some() { "erc20" } else { "native" })
    .bind(if token.is_some() { "USDC" } else { "ETH" })
    .bind(token)
    .bind(address)
    .execute(pool)
    .await
    .expect("seed payment option");

    sqlx::query(
        "INSERT INTO watched_addresses \
         (invoice_id, payment_option_id, address, chain_id, token_address, expires_at) \
         VALUES ($1, $2, $3, $4, $5, now() + interval '1 hour')",
    )
    .bind(invoice)
    .bind(option_id)
    .bind(address)
    .bind(CHAIN_TEXT)
    .bind(token)
    .execute(pool)
    .await
    .expect("seed watched address");
}

/// What one seeded discrepancy leaves behind, and what the assertions need to
/// recognise it among everything else the shared fixtures hold.
struct Seeded {
    owner: Uuid,
    /// Expected and held: must appear in neither direction.
    native: WatchKey,
    /// Expected and no longer held: the expensive direction.
    missing: WatchKey,
    /// Held and never expected: the cheap direction.
    orphan: WatchKey,
}

/// Put both directions of disagreement into the two stores.
///
/// The address keeps its native watch and loses only its token one, which is
/// the discrepancy an address-keyed comparison cannot see.
async fn seed_a_discrepancy(pg: &PgDataService, live: &RedisDataService) -> Seeded {
    let owner = seed_user(pg.pool()).await;
    let store = seed_store(pg.pool(), owner).await;
    let invoice = seed_invoice(pg.pool(), store).await;
    let invoice_id = InvoiceId::from_string(invoice.clone());

    // One address, two assets. The database expects both watched.
    let address = unique_address();
    let token = unique_address();
    seed_watch(pg.pool(), &invoice, &address, None).await;
    seed_watch(pg.pool(), &invoice, &address, Some(&token)).await;

    // The monitor holds both, as it would after accepting both watch commands.
    for tok in [None, Some(token.as_str())] {
        live.watch_address(&address, &invoice_id, &chain(), tok)
            .await
            .expect("seed a live watch");
    }

    // A live watch for an address and invoice the database has never heard of.
    let orphan_address = unique_address();
    let orphan_invoice = InvoiceId::from_string(format!("inv-{}", Uuid::new_v4()));
    live.watch_address(&orphan_address, &orphan_invoice, &chain(), None)
        .await
        .expect("seed an orphan live watch");

    // And the expensive case: the token watch disappears from the live set
    // while the database still expects it, and while the same address keeps
    // its native watch.
    let removed = live
        .unwatch_address(&address, &chain(), Some(&token))
        .await
        .expect("remove a live watch");
    assert!(removed, "the token watch must have been there to remove");

    Seeded {
        owner,
        native: WatchKey::new(&address, &invoice, chain(), None),
        missing: WatchKey::new(&address, &invoice, chain(), Some(&token)),
        orphan: WatchKey::new(&orphan_address, orphan_invoice.as_str(), chain(), None),
    }
}

/// Leave both stores as they were found. The live keys have no expiry, so an
/// orphan left behind is a permanent false positive for every later run.
async fn tidy_up(pg: &PgDataService, live: &RedisDataService, seeded: &Seeded) {
    for key in [&seeded.native, &seeded.missing, &seeded.orphan] {
        let _ = live
            .unwatch_address(&key.address, &key.chain_id, key.token_address.as_deref())
            .await;
    }
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(seeded.owner)
        .execute(pg.pool())
        .await;
}

#[tokio::test]
#[ignore = "requires a live database and live-watch store; set DATABASE_URL and TEST_REDIS_URL"]
async fn a_stale_watch_and_a_missing_watch_are_reported_as_different_things() {
    let Some(pg) = pg_service().await else {
        return;
    };
    let Some(live) = live_watches().await else {
        return;
    };

    let seeded = seed_a_discrepancy(&pg, &live).await;
    let (native_key, token_key, orphan_key) = (&seeded.native, &seeded.missing, &seeded.orphan);

    let expected = pg
        .get_expected_watches()
        .await
        .expect("read what the database expects to be watched");
    let report = reconcile_watches(expected, &live)
        .await
        .expect("compare the two sides");

    // The expensive direction: expected, not held.
    assert!(
        report.missing.contains(token_key),
        "a watch the database expects and the monitor does not hold must be \
         reported missing; missing={:?}",
        report.missing
    );
    // Reported as missing on the strength of the token, not the address: the
    // address itself is still watched, and an address-keyed comparison would
    // have found nothing wrong here.
    assert!(
        !report.missing.contains(native_key),
        "the native watch is still held and must not be reported missing"
    );

    // The cheap direction: held, not expected.
    assert!(
        report.stale.contains(orphan_key),
        "a live watch the database does not expect must be reported stale; \
         stale={:?}",
        report.stale
    );

    // Distinctly. Neither direction may leak into the other, and a watch that
    // is genuinely in both places appears in neither list.
    assert!(
        !report.stale.contains(token_key),
        "a missing watch must not also be reported stale"
    );
    assert!(
        !report.missing.contains(orphan_key),
        "a stale watch must not also be reported missing"
    );
    assert!(
        !report.stale.contains(native_key),
        "a watch both sides agree on must appear in neither direction"
    );
    let stale: HashSet<&WatchKey> = report.stale.iter().collect();
    assert!(
        !report.missing.iter().any(|m| stale.contains(m)),
        "the two directions must be disjoint"
    );

    // Counted, per direction, never as one total.
    assert!(report.missing_count() >= 1);
    assert!(report.stale_count() >= 1);
    assert!(!report.agrees());

    tidy_up(&pg, &live, &seeded).await;
}
