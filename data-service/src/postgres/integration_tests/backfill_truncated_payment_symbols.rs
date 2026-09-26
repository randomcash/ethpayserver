//! The truncated-address backfill, run against the shapes it has to tell apart.
//!
//! `asset_symbol` is read by machines, not just displayed - the analytics
//! reader groups by it and the volume capability prices against it - so a
//! typo in the join or the regex would leave a row silently wrong rather than
//! erroring. The four rows seeded here are the ones the migration has to
//! treat differently: a truncated symbol whose token is now in `tokens`,
//! one whose token still isn't, a normal symbol that must not be touched, and
//! a truncated-looking symbol with no `token_address` at all, which the
//! `token_address IS NOT NULL` guard has to leave alone.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::wallet_migration::{drop_db, migration_sql, pre_migration_db_for, seed_store};

const MIGRATION: &str = "20260921000000_backfill_truncated_payment_symbols";

async fn seed_token(pool: &PgPool, chain_id: &str, address: &str, symbol: &str) {
    sqlx::query(
        "INSERT INTO tokens (token_type, address, chain_id, symbol, decimals) \
         VALUES ('erc20', $1, $2, $3, 6)",
    )
    .bind(address)
    .bind(chain_id)
    .bind(symbol)
    .execute(pool)
    .await
    .expect("seed token");
}

async fn seed_invoice(pool: &PgPool, store_id: Uuid, label: &str) -> String {
    let invoice_id = format!("inv-{label}-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO invoices (id, store_id, currency, amount, expires_at) \
         VALUES ($1, $2, 'USD', 100, NOW() + interval '1 hour')",
    )
    .bind(&invoice_id)
    .bind(store_id)
    .execute(pool)
    .await
    .expect("seed invoice");
    invoice_id
}

/// Seed a payment_options row in the pre-migration shape: whatever
/// `asset_symbol` a broken fallback could have written.
async fn seed_payment_option(
    pool: &PgPool,
    invoice_id: &str,
    chain_id: &str,
    token_address: Option<&str>,
    asset_symbol: &str,
) -> Uuid {
    sqlx::query(
        "INSERT INTO payment_options \
         (invoice_id, payment_method_id, chain_id, asset_type, asset_symbol, \
          token_address, payment_address, amount) \
         VALUES ($1, $2, $3, $4::asset_type, $5, $6, $7, 1) RETURNING id",
    )
    .bind(invoice_id)
    .bind(format!("{asset_symbol}@{chain_id}"))
    .bind(chain_id)
    .bind(if token_address.is_some() {
        "erc20"
    } else {
        "native"
    })
    .bind(asset_symbol)
    .bind(token_address)
    .bind(format!("0x{:040x}", Uuid::new_v4().as_u128()))
    .fetch_one(pool)
    .await
    .expect("seed payment option")
    .get("id")
}

/// Seed a payments row in the same pre-migration shape.
async fn seed_payment(
    pool: &PgPool,
    invoice_id: &str,
    chain_id: &str,
    token_address: Option<&str>,
    asset_symbol: &str,
) -> Uuid {
    sqlx::query(
        "INSERT INTO payments \
         (invoice_id, chain_id, asset_type, asset_symbol, token_address, amount, tx_hash) \
         VALUES ($1, $2, $3::asset_type, $4, $5, 1, $6) RETURNING id",
    )
    .bind(invoice_id)
    .bind(chain_id)
    .bind(if token_address.is_some() {
        "erc20"
    } else {
        "native"
    })
    .bind(asset_symbol)
    .bind(token_address)
    .bind(format!("0x{:064x}", Uuid::new_v4().as_u128()))
    .fetch_one(pool)
    .await
    .expect("seed payment")
    .get("id")
}

async fn symbol_of_payment(pool: &PgPool, id: Uuid) -> String {
    sqlx::query("SELECT asset_symbol FROM payments WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read payment")
        .get("asset_symbol")
}

async fn symbol_of_payment_option(pool: &PgPool, id: Uuid) -> String {
    sqlx::query("SELECT asset_symbol FROM payment_options WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read payment option")
        .get("asset_symbol")
}

/// Seed one case: an invoice carrying a payment and a payment option in the
/// same pre-migration shape, so both UPDATEs in the migration see it.
async fn seed_case(
    pool: &PgPool,
    store: Uuid,
    chain: &str,
    label: &str,
    token_address: Option<&str>,
    asset_symbol: &str,
) -> (Uuid, Uuid) {
    let invoice = seed_invoice(pool, store, label).await;
    let option = seed_payment_option(pool, &invoice, chain, token_address, asset_symbol).await;
    let payment = seed_payment(pool, &invoice, chain, token_address, asset_symbol).await;
    (option, payment)
}

async fn assert_resolved(pool: &PgPool, case: (Uuid, Uuid), expected: &str, why: &str) {
    assert_eq!(
        symbol_of_payment_option(pool, case.0).await,
        expected,
        "{why}"
    );
    assert_eq!(symbol_of_payment(pool, case.1).await, expected, "{why}");
}

/// The migration resolves a truncated symbol where `tokens` now has the
/// contract, falls back to `ERC20` where it still doesn't, and leaves both a
/// normal symbol and a truncated-looking one with no `token_address` alone -
/// on both `payments` and `payment_options`.
#[tokio::test]
#[ignore]
async fn migration_resolves_known_tokens_and_falls_back_for_the_rest() {
    let Some((pool, name, server)) = pre_migration_db_for(MIGRATION, "backfill").await else {
        return;
    };

    let (_, store) = seed_store(&pool, "backfill").await;
    // Sepolia, the chain this bug was found on.
    const CHAIN: &str = "eip155:11155111";
    const KNOWN_TOKEN: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    seed_token(&pool, CHAIN, KNOWN_TOKEN, "USDC").await;

    let resolvable = seed_case(
        &pool,
        store,
        CHAIN,
        "resolvable",
        Some(&KNOWN_TOKEN.to_lowercase()),
        "0x1c7d4b...",
    )
    .await;
    let unknown = seed_case(
        &pool,
        store,
        CHAIN,
        "unknown",
        Some("0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead"),
        "0xdeadde...",
    )
    .await;
    let native = seed_case(&pool, store, CHAIN, "native", None, "ETH").await;
    let guard = seed_case(&pool, store, CHAIN, "guard", None, "0xfeedfa...").await;

    pool.execute_migration(MIGRATION).await;

    assert_resolved(
        &pool,
        resolvable,
        "USDC",
        "a truncated row whose token is now in `tokens` must resolve to the \
         real symbol",
    )
    .await;
    assert_resolved(
        &pool,
        unknown,
        "ERC20",
        "a truncated row whose token is still unknown must fall back to \
         ERC20, the same rule #215 applies to new rows - never re-derive from \
         the address",
    )
    .await;
    assert_resolved(&pool, native, "ETH", "a normal symbol must not be touched").await;
    assert_resolved(
        &pool,
        guard,
        "0xfeedfa...",
        "a truncated-looking symbol with no token_address has nothing to \
         resolve against and must be left alone by the IS NOT NULL guard",
    )
    .await;

    drop_db(pool, &name, &server).await;
}

/// Small helper so the test reads as "seed, migrate, assert" like the
/// migrations it sits next to.
trait ApplyMigration {
    async fn execute_migration(&self, stem: &str);
}

impl ApplyMigration for PgPool {
    async fn execute_migration(&self, stem: &str) {
        use sqlx::Executor;
        self.execute(migration_sql(stem).as_str())
            .await
            .unwrap_or_else(|e| panic!("apply {stem}: {e}"));
    }
}
