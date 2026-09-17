//! The chain-family migration, run against data in the shape it will find.
//!
//! What this migration changes is not a column. It re-keys `store_wallets` -
//! the table that decides which key a store collects on - and adds two
//! composite foreign keys over live routing. The failure mode of getting that
//! wrong is not an error at deploy time; it is a store that comes back up
//! pointing at a different wallet than it went down with.
//!
//! So the properties asserted here are about what *survived*: every wallet is
//! Ethereum (it has to be - nothing else could exist yet), every override still
//! names the wallet it named before, and every payment method still derives
//! from the key it derived from. Plus the guarantee the migration exists to
//! add, checked from the outside: a payment method can no longer be pinned to a
//! wallet from another family.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::wallet_migration::{drop_db, migration_sql, pre_migration_db_for, seed_store};

const MIGRATION: &str = "20260917000000_wallet_chain_namespace";

/// Seed a wallet in the shape this migration finds: no namespace column.
async fn seed_wallet(pool: &PgPool, user_id: Uuid, xpub: &str, primary: bool) -> Uuid {
    sqlx::query("INSERT INTO wallets (user_id, xpub, is_primary) VALUES ($1, $2, $3) RETURNING id")
        .bind(user_id)
        .bind(xpub)
        .bind(primary)
        .fetch_one(pool)
        .await
        .expect("seed wallet")
        .get("id")
}

/// Seed a payment method pinned to a wallet, post-account-wallets shape.
async fn seed_pinned_method(
    pool: &PgPool,
    store_id: Uuid,
    chain_id: &str,
    symbol: &str,
    wallet_id: Uuid,
) -> Uuid {
    sqlx::query(
        "INSERT INTO store_payment_methods \
         (store_id, chain_id, token_address, asset_symbol, decimals, wallet_id) \
         VALUES ($1, $2, NULL, $3, 18, $4) RETURNING id",
    )
    .bind(store_id)
    .bind(chain_id)
    .bind(symbol)
    .bind(wallet_id)
    .fetch_one(pool)
    .await
    .expect("seed payment method")
    .get("id")
}

/// Nothing about where money goes may change when this migration runs.
///
/// Every wallet becomes `eip155` - which is not an assumption but the only
/// truth available, since no other family could have been registered before
/// this migration existed - and every override and every pin comes through
/// naming exactly what it named before. A store whose override moved, or whose
/// pinned method came back inheriting, would be collecting somewhere its owner
/// never chose.
#[tokio::test]
#[ignore]
async fn the_migration_changes_no_routing() {
    let Some((pool, name, server)) = pre_migration_db_for(MIGRATION, "ns_routing").await else {
        return;
    };

    let (user, store) = seed_store(&pool, "ns").await;
    let primary = seed_wallet(&pool, user, "xpub-primary", true).await;
    let pinned_to = seed_wallet(&pool, user, "xpub-pinned", false).await;

    // One store pinned to the non-primary wallet, with one method pinned to it
    // too and one method inheriting.
    sqlx::query("INSERT INTO store_wallets (store_id, wallet_id) VALUES ($1, $2)")
        .bind(store)
        .bind(pinned_to)
        .execute(&pool)
        .await
        .expect("seed override");
    let pinned_method = seed_pinned_method(&pool, store, "eip155:1", "ETH", pinned_to).await;
    let inheriting: Uuid = sqlx::query(
        "INSERT INTO store_payment_methods \
         (store_id, chain_id, token_address, asset_symbol, decimals, wallet_id) \
         VALUES ($1, 'eip155:137', NULL, 'MATIC', 18, NULL) RETURNING id",
    )
    .bind(store)
    .fetch_one(&pool)
    .await
    .expect("seed inheriting method")
    .get("id");

    pool.execute_migration(MIGRATION).await;

    // Every wallet is Ethereum, and the override is filed under that family.
    let families: Vec<String> = sqlx::query("SELECT namespace FROM wallets ORDER BY xpub")
        .fetch_all(&pool)
        .await
        .expect("read namespaces")
        .iter()
        .map(|r| r.get::<String, _>("namespace"))
        .collect();
    assert_eq!(
        families,
        vec!["eip155".to_string(), "eip155".to_string()],
        "the backfill is the only truth available: no other family could have \
         been registered before this migration existed"
    );

    let override_row =
        sqlx::query("SELECT wallet_id, namespace FROM store_wallets WHERE store_id = $1")
            .bind(store)
            .fetch_one(&pool)
            .await
            .expect("the override survived");
    assert_eq!(override_row.get::<Uuid, _>("wallet_id"), pinned_to);
    assert_eq!(override_row.get::<String, _>("namespace"), "eip155");

    // The pinned method is still pinned to the same wallet; the inheriting one
    // is still inheriting. Either flipping would move where money goes.
    let still_pinned: Option<Uuid> =
        sqlx::query("SELECT wallet_id FROM store_payment_methods WHERE id = $1")
            .bind(pinned_method)
            .fetch_one(&pool)
            .await
            .expect("read pinned method")
            .get("wallet_id");
    assert_eq!(still_pinned, Some(pinned_to));

    let still_inheriting: Option<Uuid> =
        sqlx::query("SELECT wallet_id FROM store_payment_methods WHERE id = $1")
            .bind(inheriting)
            .fetch_one(&pool)
            .await
            .expect("read inheriting method")
            .get("wallet_id");
    assert_eq!(still_inheriting, None);

    // And the primary is untouched.
    let primary_after: Uuid =
        sqlx::query("SELECT id FROM wallets WHERE user_id = $1 AND is_primary")
            .bind(user)
            .fetch_one(&pool)
            .await
            .expect("exactly one primary")
            .get("id");
    assert_eq!(primary_after, primary);

    drop_db(pool, &name, &server).await;
}

/// After the migration, a payment method cannot be pinned to a wallet from
/// another family - the database refuses it.
///
/// The property the composite foreign key exists for, asserted from outside
/// the Rust that is supposed to prevent it. Resolution filters by namespace in
/// SQL, but a pin is a column: a `tron:` method holding an `eip155` wallet id
/// would resolve to that wallet perfectly happily, and every query that walks
/// the chain would have to remember to check. This makes it unrepresentable
/// instead.
#[tokio::test]
#[ignore]
async fn after_the_migration_a_cross_family_pin_is_refused() {
    let Some((pool, name, server)) = pre_migration_db_for(MIGRATION, "ns_pin").await else {
        return;
    };

    let (user, store) = seed_store(&pool, "ns_pin").await;
    let evm = seed_wallet(&pool, user, "xpub-evm", true).await;

    pool.execute_migration(MIGRATION).await;

    // Same family: accepted, exactly as before.
    seed_pinned_method(&pool, store, "eip155:1", "ETH", evm).await;

    // Different family: refused by the schema, not by a query that had to
    // remember.
    let refused = sqlx::query(
        "INSERT INTO store_payment_methods \
         (store_id, chain_id, token_address, asset_symbol, decimals, wallet_id) \
         VALUES ($1, 'tron:728126428', NULL, 'USDT', 6, $2)",
    )
    .bind(store)
    .bind(evm)
    .execute(&pool)
    .await;

    match refused {
        Err(sqlx::Error::Database(db)) => assert!(
            db.is_foreign_key_violation(),
            "wrong error for a cross-family pin: {db}"
        ),
        other => panic!("a tron method was pinned to an ethereum wallet: {other:?}"),
    }

    // An unpinned tron method is still allowed - it simply resolves to nothing
    // until the account registers a tron key. Refusing it here would make the
    // row unrepresentable rather than unpayable, and a merchant could not
    // configure a chain before adding its key.
    sqlx::query(
        "INSERT INTO store_payment_methods \
         (store_id, chain_id, token_address, asset_symbol, decimals, wallet_id) \
         VALUES ($1, 'tron:728126428', NULL, 'USDT', 6, NULL)",
    )
    .bind(store)
    .execute(&pool)
    .await
    .expect("an unpinned tron method is a legitimate row");

    drop_db(pool, &name, &server).await;
}

/// A store can hold one override per family, and setting one does not disturb
/// the other.
///
/// The reason `store_wallets` was re-keyed rather than left alone. With
/// `store_id` as the primary key, writing a Tron override REPLACED the
/// Ethereum one, and every EVM method following that override silently moved
/// to the account's Ethereum primary.
#[tokio::test]
#[ignore]
async fn after_the_migration_a_store_holds_one_override_per_family() {
    let Some((pool, name, server)) = pre_migration_db_for(MIGRATION, "ns_two").await else {
        return;
    };

    let (user, store) = seed_store(&pool, "ns_two").await;
    let evm = seed_wallet(&pool, user, "xpub-evm", true).await;

    sqlx::query("INSERT INTO store_wallets (store_id, wallet_id) VALUES ($1, $2)")
        .bind(store)
        .bind(evm)
        .execute(&pool)
        .await
        .expect("seed the ethereum override");

    pool.execute_migration(MIGRATION).await;

    let tron: Uuid = sqlx::query(
        "INSERT INTO wallets (user_id, namespace, xpub, is_primary) \
         VALUES ($1, 'tron', 'xpub-tron', true) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .expect("register a tron wallet")
    .get("id");

    sqlx::query(
        "INSERT INTO store_wallets (store_id, wallet_id, namespace) VALUES ($1, $2, 'tron')",
    )
    .bind(store)
    .bind(tron)
    .execute(&pool)
    .await
    .expect("a tron override is a second row, not a replacement");

    let rows: Vec<(String, Uuid)> = sqlx::query(
        "SELECT namespace, wallet_id FROM store_wallets WHERE store_id = $1 ORDER BY namespace",
    )
    .bind(store)
    .fetch_all(&pool)
    .await
    .expect("read overrides")
    .iter()
    .map(|r| (r.get("namespace"), r.get("wallet_id")))
    .collect();

    assert_eq!(
        rows,
        vec![("eip155".to_string(), evm), ("tron".to_string(), tron)],
        "pinning a tron wallet replaced the store's ethereum override"
    );

    drop_db(pool, &name, &server).await;
}

/// Small helper so each test reads as "seed, migrate, assert".
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
