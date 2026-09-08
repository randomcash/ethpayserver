//! Account wallet repository implementation (RCS-234).

use async_trait::async_trait;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::PgDataService;
use crate::{
    RepositoryError, RepositoryResult, Wallet, WalletReader, WalletWriter, sqlx_to_repo_error,
};

/// Every column of `wallets`, in the order `row_to_wallet` reads them.
const WALLET_COLUMNS: &str = "id, user_id, xpub, derivation_index, name, is_primary, created_at";

/// The same list, aliased. `stores` also has an `id`, so an unqualified list
/// in a query that joins the two is ambiguous and Postgres rejects it.
const WALLET_COLUMNS_W: &str = "w.id, w.user_id, w.xpub, w.derivation_index, w.name, \
     w.is_primary, w.created_at";

/// Resolve the wallet a store derives from: its override, else the owner's
/// primary. `$1` is the store id.
///
/// Spelled once and reused, because the same chain has to be walked by reads,
/// by allocation and by the migration. A second spelling that drifts is a
/// store quietly collecting on a key the UI never showed.
pub(super) const STORE_WALLET_RESOLUTION: &str = "COALESCE(
        (SELECT sw.wallet_id FROM store_wallets sw WHERE sw.store_id = s.id),
        (SELECT p.id FROM wallets p WHERE p.user_id = s.owner_id AND p.is_primary)
    )";

fn row_to_wallet(row: &sqlx::postgres::PgRow) -> Wallet {
    Wallet {
        id: row.get("id"),
        user_id: row.get("user_id"),
        xpub: row.get("xpub"),
        derivation_index: row.get("derivation_index"),
        name: row.get("name"),
        is_primary: row.get("is_primary"),
        created_at: row.get("created_at"),
    }
}

/// Serialise everything that touches one account's primary flag.
///
/// `is_primary` is decided with `NOT EXISTS (... WHERE user_id = $1)` while the
/// insert's `ON CONFLICT` targets `(user_id, xpub)` - a different index from
/// the partial unique one that enforces the primary. Two concurrent creates on
/// a fresh account therefore both evaluate that to true, and one loses on an
/// index it was not conflicting against, surfacing as a bare unique violation.
/// That is reachable from the ordinary "enable ETH, enable USDC" flow, so it is
/// locked rather than retried.
pub(super) async fn lock_account(conn: &mut PgConnection, user_id: Uuid) -> RepositoryResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('rcs234:user'), hashtext($1::text))")
        .bind(user_id)
        .execute(conn)
        .await
        .map_err(sqlx_to_repo_error)?;
    Ok(())
}

/// Serialise everything that registers a given xpub, across all accounts.
///
/// There is no global unique index on `xpub` - the migration cannot create one,
/// because a key already shared by two accounts has no correct owner to award
/// it to - so cross-account exclusivity is checked rather than constrained. A
/// check alone races: two accounts registering the same key at once would both
/// find it free. This closes that window.
///
/// Always taken AFTER `lock_account`, so the two never deadlock against each
/// other.
pub(super) async fn lock_xpub(conn: &mut PgConnection, xpub: &str) -> RepositoryResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('rcs234:xpub'), hashtext($1))")
        .bind(xpub)
        .execute(conn)
        .await
        .map_err(sqlx_to_repo_error)?;
    Ok(())
}

/// Take the account lock for whoever owns `store_id`.
///
/// Everything that changes where an account's money is collected takes this
/// lock first, so those writers are ordered against each other rather than
/// against whichever row they happen to touch first. Rotation writes payment
/// methods then the override; setting an override writes the override then the
/// payment methods. Without a lock in front, those two orders deadlock - which
/// Postgres does detect and abort, but as a 500 on a request that had nothing
/// wrong with it.
async fn lock_store_account(conn: &mut PgConnection, store_id: Uuid) -> RepositoryResult<()> {
    let owner = sqlx::query("SELECT owner_id FROM stores WHERE id = $1")
        .bind(store_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(sqlx_to_repo_error)?
        .ok_or_else(|| RepositoryError::NotFound("store not found".into()))?;

    lock_account(conn, owner.get("owner_id")).await
}

/// Refuse an xpub that another account already holds.
///
/// Two accounts deriving from one key is the same collision as two counters on
/// one key, reached from the other direction: each counts independently and
/// both issue index 0, 1, 2 to different merchants' customers. Within an
/// account `idx_account_wallets_user_xpub` makes it impossible; across accounts
/// this is the enforcement, and it only holds because the caller is holding
/// `lock_xpub`.
pub(super) async fn reject_if_another_account_holds(
    conn: &mut PgConnection,
    user_id: Uuid,
    xpub: &str,
) -> RepositoryResult<()> {
    let taken = sqlx::query("SELECT 1 AS x FROM wallets WHERE xpub = $1 AND user_id <> $2 LIMIT 1")
        .bind(xpub)
        .bind(user_id)
        .fetch_optional(conn)
        .await
        .map_err(sqlx_to_repo_error)?;

    if taken.is_some() {
        return Err(RepositoryError::Conflict(
            "this xpub is already registered to another account".into(),
        ));
    }
    Ok(())
}

/// Find or create the wallet holding `xpub` for `user_id`, inside a caller's
/// transaction that already holds both locks.
pub(super) async fn upsert_wallet(
    conn: &mut PgConnection,
    user_id: Uuid,
    xpub: &str,
    name: Option<&str>,
) -> RepositoryResult<Wallet> {
    // `DO UPDATE` rather than `DO NOTHING`: the latter returns no row on
    // conflict and the caller would see a spurious "not found" for a wallet
    // that plainly exists.
    let row = sqlx::query(&format!(
        r#"
        INSERT INTO wallets (user_id, xpub, name, is_primary)
        VALUES ($1, $2, $3, NOT EXISTS (SELECT 1 FROM wallets WHERE user_id = $1))
        ON CONFLICT (user_id, xpub) DO UPDATE
            SET name = COALESCE(EXCLUDED.name, wallets.name)
        RETURNING {WALLET_COLUMNS}
        "#
    ))
    .bind(user_id)
    .bind(xpub)
    .bind(name)
    .fetch_one(conn)
    .await
    .map_err(sqlx_to_repo_error)?;

    Ok(row_to_wallet(&row))
}

impl PgDataService {
    /// Find or create the account wallet holding `xpub` for a store's owner.
    ///
    /// Payment methods are still configured by pasting an xpub, so this is
    /// where that xpub becomes a wallet - and where the same cross-account
    /// refusal applies, since configuring a method is another way in.
    pub(super) async fn wallet_for_store_xpub(
        &self,
        store_id: Uuid,
        xpub: &str,
    ) -> RepositoryResult<Uuid> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;

        let owner = sqlx::query("SELECT owner_id FROM stores WHERE id = $1")
            .bind(store_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?
            .ok_or_else(|| RepositoryError::NotFound("store not found".into()))?;
        let owner_id: Uuid = owner.get("owner_id");

        lock_account(&mut tx, owner_id).await?;
        lock_xpub(&mut tx, xpub).await?;
        reject_if_another_account_holds(&mut tx, owner_id, xpub).await?;

        let wallet = upsert_wallet(&mut tx, owner_id, xpub, None).await?;
        tx.commit().await.map_err(sqlx_to_repo_error)?;
        Ok(wallet.id)
    }
}

#[async_trait]
impl WalletReader for PgDataService {
    async fn get_wallet(&self, wallet_id: Uuid) -> RepositoryResult<Option<Wallet>> {
        let row = sqlx::query(&format!(
            "SELECT {WALLET_COLUMNS} FROM wallets WHERE id = $1"
        ))
        .bind(wallet_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_wallet))
    }

    async fn list_wallets(&self, user_id: Uuid) -> RepositoryResult<Vec<Wallet>> {
        let rows = sqlx::query(&format!(
            "SELECT {WALLET_COLUMNS} FROM wallets WHERE user_id = $1 \
             ORDER BY is_primary DESC, created_at ASC, id ASC"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_wallet).collect())
    }

    async fn get_primary_wallet(&self, user_id: Uuid) -> RepositoryResult<Option<Wallet>> {
        let row = sqlx::query(&format!(
            "SELECT {WALLET_COLUMNS} FROM wallets WHERE user_id = $1 AND is_primary"
        ))
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_wallet))
    }

    async fn resolve_store_wallet(&self, store_id: Uuid) -> RepositoryResult<Option<Wallet>> {
        // Override first, primary second, in one round trip. Two queries would
        // let a `set_primary_wallet` land between them and resolve a store to
        // a wallet that was never either of its answers.
        let row = sqlx::query(&format!(
            r#"
            SELECT {WALLET_COLUMNS_W}
            FROM wallets w
            JOIN stores s ON s.id = $1
            WHERE w.id = {STORE_WALLET_RESOLUTION}
            "#
        ))
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_wallet))
    }

    async fn get_store_wallet_override(&self, store_id: Uuid) -> RepositoryResult<Option<Uuid>> {
        let row = sqlx::query("SELECT wallet_id FROM store_wallets WHERE store_id = $1")
            .bind(store_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(row.map(|r| r.get("wallet_id")))
    }
}

#[async_trait]
impl WalletWriter for PgDataService {
    async fn create_wallet(
        &self,
        user_id: Uuid,
        xpub: &str,
        name: Option<&str>,
    ) -> RepositoryResult<Wallet> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;

        lock_account(&mut tx, user_id).await?;
        lock_xpub(&mut tx, xpub).await?;
        reject_if_another_account_holds(&mut tx, user_id, xpub).await?;

        // Re-adding an xpub the account already holds returns the existing row.
        // A second row would be a second counter on one key, which is the
        // collision this module exists to prevent. The first wallet on an
        // account becomes its primary, so a merchant with one wallet never has
        // to meet the concept.
        let wallet = upsert_wallet(&mut tx, user_id, xpub, name).await?;

        tx.commit().await.map_err(sqlx_to_repo_error)?;
        Ok(wallet)
    }

    async fn rename_wallet(&self, wallet_id: Uuid, name: Option<&str>) -> RepositoryResult<Wallet> {
        let row = sqlx::query(&format!(
            "UPDATE wallets SET name = $2 WHERE id = $1 RETURNING {WALLET_COLUMNS}"
        ))
        .bind(wallet_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        row.as_ref()
            .map(row_to_wallet)
            .ok_or_else(|| RepositoryError::NotFound("wallet not found".into()))
    }

    async fn set_primary_wallet(&self, user_id: Uuid, wallet_id: Uuid) -> RepositoryResult<Wallet> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;
        lock_account(&mut tx, user_id).await?;

        // Demote before promoting. `idx_account_wallets_one_primary` is an
        // immediate unique index, so promoting first would fail against the
        // outgoing primary even inside a transaction.
        sqlx::query("UPDATE wallets SET is_primary = FALSE WHERE user_id = $1 AND is_primary")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;

        // `user_id = $1` in the promotion is the tenancy check: without it a
        // caller could hand over someone else's wallet id and take it over.
        let row = sqlx::query(&format!(
            "UPDATE wallets SET is_primary = TRUE WHERE id = $2 AND user_id = $1 \
             RETURNING {WALLET_COLUMNS}"
        ))
        .bind(user_id)
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        let wallet = row
            .as_ref()
            .map(row_to_wallet)
            .ok_or_else(|| RepositoryError::NotFound("wallet not found".into()))?;

        tx.commit().await.map_err(sqlx_to_repo_error)?;
        Ok(wallet)
    }

    async fn delete_wallet(&self, wallet_id: Uuid) -> RepositoryResult<()> {
        // `store_payment_methods.wallet_id` and `store_wallets.wallet_id` are
        // ON DELETE RESTRICT, so a wallet in use raises a foreign key
        // violation here. `payment_options.wallet_id` is ON DELETE SET NULL by
        // contrast: history must never be the reason a wallet cannot be
        // removed. Translate the violation rather than let it surface as a
        // 500 - "in use" is something the caller can act on.
        let result = sqlx::query("DELETE FROM wallets WHERE id = $1")
            .bind(wallet_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                if let sqlx::Error::Database(ref db) = e
                    && db.is_foreign_key_violation()
                {
                    return RepositoryError::Conflict(
                        "wallet is still used by a store or payment method".into(),
                    );
                }
                sqlx_to_repo_error(e)
            })?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound("wallet not found".into()));
        }

        Ok(())
    }

    async fn set_store_wallet(&self, store_id: Uuid, wallet_id: Uuid) -> RepositoryResult<()> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;
        lock_store_account(&mut tx, store_id).await?;

        // The wallet has to belong to the store's owner. Enforced in the
        // predicate rather than by a prior SELECT so there is no window in
        // which ownership changes between check and write; zero rows inserted
        // means the pairing was not legitimate.
        let result = sqlx::query(
            r#"
            INSERT INTO store_wallets (store_id, wallet_id)
            SELECT s.id, w.id
            FROM stores s
            JOIN wallets w ON w.user_id = s.owner_id
            WHERE s.id = $1 AND w.id = $2
            ON CONFLICT (store_id) DO UPDATE SET wallet_id = EXCLUDED.wallet_id
            "#,
        )
        .bind(store_id)
        .bind(wallet_id)
        .execute(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(
                "store or wallet not found, or the wallet belongs to another account".into(),
            ));
        }

        // Release the store's payment methods so the override actually decides
        // where money goes.
        //
        // Without this the endpoint is cosmetic: the override row changes, the
        // GET reports the new wallet, and the next invoice still derives from
        // whatever each method was pinned to. Every method the migration
        // touched is pinned - that is what kept routing identical on day one -
        // so "give this store its own wallet" has to mean the store's methods
        // now follow the store. They stay unpinned afterwards, so a later
        // change of primary reaches them too.
        sqlx::query("UPDATE store_payment_methods SET wallet_id = NULL WHERE store_id = $1")
            .bind(store_id)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;

        tx.commit().await.map_err(sqlx_to_repo_error)?;
        Ok(())
    }

    async fn clear_store_wallet(&self, store_id: Uuid) -> RepositoryResult<()> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;
        lock_store_account(&mut tx, store_id).await?;

        // Idempotent: no override is the state the caller asked for, so
        // clearing twice is not an error.
        sqlx::query("DELETE FROM store_wallets WHERE store_id = $1")
            .bind(store_id)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;

        // Unpin too, for the same reason `set_store_wallet` does: otherwise
        // "stop overriding" would leave methods pinned to the wallet the
        // override used to name, and the store would keep deriving from it
        // while reporting the primary.
        sqlx::query("UPDATE store_payment_methods SET wallet_id = NULL WHERE store_id = $1")
            .bind(store_id)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;

        tx.commit().await.map_err(sqlx_to_repo_error)?;
        Ok(())
    }

    async fn next_derivation_index(&self, wallet_id: Uuid) -> RepositoryResult<i32> {
        // One statement, deliberately. `UPDATE ... RETURNING` takes a row lock
        // on the wallet for the duration, so concurrent allocations serialise
        // and each caller sees a distinct index; a SELECT followed by an
        // UPDATE would hand the same index to both under READ COMMITTED.
        //
        // The pre-RCS-234 code did the same thing on store_payment_methods, so
        // the statement was never the problem: the counter was. Two methods
        // sharing an xpub were two rows, each perfectly serialised against
        // itself and not at all against the other. Locking the wallet is what
        // makes them contend, which is the point.
        let row = sqlx::query(
            r#"
            UPDATE wallets
            SET derivation_index = derivation_index + 1
            WHERE id = $1
            RETURNING derivation_index - 1 AS current_index
            "#,
        )
        .bind(wallet_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(r.get("current_index")),
            None => Err(RepositoryError::NotFound("wallet not found".into())),
        }
    }
}
