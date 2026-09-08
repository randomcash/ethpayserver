//! Account wallet repository implementation (RCS-234).

use async_trait::async_trait;
use sqlx::Row;
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
            WHERE w.id = COALESCE(
                (SELECT sw.wallet_id FROM store_wallets sw WHERE sw.store_id = s.id),
                (SELECT p.id FROM wallets p WHERE p.user_id = s.owner_id AND p.is_primary)
            )
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
        // ON CONFLICT on (user_id, xpub) rather than an insert that fails:
        // re-adding an xpub the account already holds is a no-op, not a second
        // counter on the same key. `DO UPDATE` (not `DO NOTHING`) so the row
        // comes back either way - `DO NOTHING` returns nothing on conflict and
        // the caller would see a spurious "not found".
        //
        // The first wallet on an account becomes its primary, so a merchant
        // who adds exactly one wallet never has to think about the concept.
        let row = sqlx::query(&format!(
            r#"
            INSERT INTO wallets (user_id, xpub, name, is_primary)
            VALUES (
                $1, $2, $3,
                NOT EXISTS (SELECT 1 FROM wallets WHERE user_id = $1)
            )
            ON CONFLICT (user_id, xpub) DO UPDATE
                SET name = COALESCE(EXCLUDED.name, wallets.name)
            RETURNING {WALLET_COLUMNS}
            "#
        ))
        .bind(user_id)
        .bind(xpub)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row_to_wallet(&row))
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
        // The FKs from store_payment_methods and store_wallets are ON DELETE
        // RESTRICT, so a wallet still in use raises a foreign key violation
        // here. Translate it rather than let it surface as a 500: "in use" is
        // a thing the caller can act on.
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
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(
                "store or wallet not found, or the wallet belongs to another account".into(),
            ));
        }

        Ok(())
    }

    async fn clear_store_wallet(&self, store_id: Uuid) -> RepositoryResult<()> {
        // Idempotent: no override is the state the caller asked for, so
        // clearing twice is not an error.
        sqlx::query("DELETE FROM store_wallets WHERE store_id = $1")
            .bind(store_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

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
