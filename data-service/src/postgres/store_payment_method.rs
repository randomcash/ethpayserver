//! Store payment method repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{RepositoryError, RepositoryResult, sqlx_to_repo_error};
use types::StorePaymentMethod;
use types::{StorePaymentMethodReader, StorePaymentMethodWriter};

/// The projection every read uses.
///
/// `xpub` and `derivation_index` come from `wallets`, not from the payment
/// method: since RCS-234 the method only points at a wallet. Reading them
/// through this join is what keeps `StorePaymentMethod` the same shape for
/// callers while making it impossible for two methods on one key to disagree
/// about how far the counter has got.
const METHOD_COLUMNS: &str = "pm.id, pm.store_id, pm.chain_id, pm.token_address, \
     pm.asset_symbol, pm.decimals, pm.wallet_id, w.xpub, w.derivation_index, \
     pm.enabled, pm.created_at";

/// `FROM` clause pairing a payment method with its wallet.
const METHOD_FROM: &str = "FROM store_payment_methods pm JOIN wallets w ON w.id = pm.wallet_id";

fn row_to_payment_method(row: &sqlx::postgres::PgRow) -> StorePaymentMethod {
    let decimals: i16 = row.get("decimals");
    StorePaymentMethod {
        id: row.get("id"),
        store_id: row.get("store_id"),
        chain_id: row.get::<i64, _>("chain_id") as u64,
        token_address: row.get("token_address"),
        asset_symbol: row.get("asset_symbol"),
        decimals: decimals as u8,
        wallet_id: row.get("wallet_id"),
        xpub: row.get("xpub"),
        derivation_index: row.get("derivation_index"),
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
    }
}

#[async_trait]
impl StorePaymentMethodReader for PgDataService {
    async fn get_payment_methods(
        &self,
        store_id: Uuid,
    ) -> RepositoryResult<Vec<StorePaymentMethod>> {
        let rows = sqlx::query(&format!(
            "SELECT {METHOD_COLUMNS} {METHOD_FROM} \
             WHERE pm.store_id = $1 ORDER BY pm.created_at"
        ))
        .bind(store_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_payment_method).collect())
    }

    async fn get_enabled_payment_methods(
        &self,
        store_id: Uuid,
    ) -> RepositoryResult<Vec<StorePaymentMethod>> {
        let rows = sqlx::query(&format!(
            "SELECT {METHOD_COLUMNS} {METHOD_FROM} \
             WHERE pm.store_id = $1 AND pm.enabled = true ORDER BY pm.created_at"
        ))
        .bind(store_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_payment_method).collect())
    }

    async fn get_payment_method(&self, id: Uuid) -> RepositoryResult<Option<StorePaymentMethod>> {
        let row = sqlx::query(&format!(
            "SELECT {METHOD_COLUMNS} {METHOD_FROM} WHERE pm.id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_payment_method))
    }

    async fn get_payment_method_by_chain(
        &self,
        store_id: Uuid,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<Option<StorePaymentMethod>> {
        let row = match token_address {
            Some(addr) => {
                sqlx::query(&format!(
                    "SELECT {METHOD_COLUMNS} {METHOD_FROM} \
                     WHERE pm.store_id = $1 AND pm.chain_id = $2 AND pm.token_address = $3"
                ))
                .bind(store_id)
                .bind(chain_id as i64)
                .bind(addr)
                .fetch_optional(&self.pool)
                .await
            }
            None => {
                sqlx::query(&format!(
                    "SELECT {METHOD_COLUMNS} {METHOD_FROM} \
                     WHERE pm.store_id = $1 AND pm.chain_id = $2 AND pm.token_address IS NULL"
                ))
                .bind(store_id)
                .bind(chain_id as i64)
                .fetch_optional(&self.pool)
                .await
            }
        }
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_payment_method))
    }

    async fn find_by_asset_symbol(
        &self,
        store_id: Uuid,
        asset_symbol: &str,
    ) -> RepositoryResult<Vec<StorePaymentMethod>> {
        let rows = sqlx::query(&format!(
            "SELECT {METHOD_COLUMNS} {METHOD_FROM} \
             WHERE pm.store_id = $1 AND pm.asset_symbol = $2 AND pm.enabled = true \
             ORDER BY pm.created_at"
        ))
        .bind(store_id)
        .bind(asset_symbol)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_payment_method).collect())
    }
}

#[async_trait]
impl StorePaymentMethodWriter for PgDataService {
    async fn create_payment_method(
        &self,
        store_id: Uuid,
        chain_id: u64,
        token_address: Option<&str>,
        asset_symbol: &str,
        decimals: u8,
        xpub: &str,
    ) -> RepositoryResult<StorePaymentMethod> {
        let wallet_id = self.wallet_for_store_xpub(store_id, xpub).await?;

        sqlx::query(
            r#"
            INSERT INTO store_payment_methods
                (store_id, chain_id, token_address, asset_symbol, decimals, wallet_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (store_id, chain_id, token_address) DO UPDATE
            SET wallet_id = $6, asset_symbol = $4, decimals = $5, enabled = true
            RETURNING id
            "#,
        )
        .bind(store_id)
        .bind(chain_id as i64)
        .bind(token_address)
        .bind(asset_symbol)
        .bind(decimals as i16)
        .bind(wallet_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        // Re-read through the join rather than RETURNING the inserted row: the
        // xpub and index the caller expects live on `wallets`, and INSERT ...
        // RETURNING cannot reach across to them.
        self.get_payment_method_by_chain(store_id, chain_id, token_address)
            .await?
            .ok_or_else(|| RepositoryError::NotFound("payment method not found".into()))
    }

    async fn update_payment_method(
        &self,
        id: Uuid,
        enabled: Option<bool>,
        xpub: Option<&str>,
    ) -> RepositoryResult<StorePaymentMethod> {
        // Resolving the xpub to a wallet needs the store, so the method has to
        // exist first. This also gives `update` its not-found error rather
        // than letting a zero-row UPDATE report it later.
        let existing = self
            .get_payment_method(id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound("payment method not found".into()))?;

        let wallet_id = match xpub {
            Some(x) => Some(self.wallet_for_store_xpub(existing.store_id, x).await?),
            None => None,
        };

        // Repointing at a different wallet does NOT reset a counter, unlike
        // the old per-method xpub swap: the new wallet already knows how far
        // its own key has been used, which is the whole reason the counter
        // moved (RCS-234). Resetting anything here would re-issue addresses.
        sqlx::query(
            r#"
            UPDATE store_payment_methods
            SET enabled = COALESCE($2, enabled),
                wallet_id = COALESCE($3, wallet_id)
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(enabled)
        .bind(wallet_id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        self.get_payment_method(id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound("payment method not found".into()))
    }

    async fn delete_payment_method(&self, id: Uuid) -> RepositoryResult<()> {
        let result = sqlx::query("DELETE FROM store_payment_methods WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound("payment method not found".into()));
        }

        Ok(())
    }

    async fn next_derivation_index(&self, id: Uuid) -> RepositoryResult<i32> {
        // The counter is on the wallet, so the lock is taken on the wallet.
        // Two payment methods of the same store - ETH and USDC on one xpub,
        // the ordinary configuration - now contend for one row and get
        // distinct indices. Before RCS-234 each held its own counter and both
        // happily returned the same number, which is how one address ended up
        // serving several methods.
        //
        // Still a single `UPDATE ... RETURNING`: atomic, and never a SELECT
        // followed by an UPDATE.
        let row = sqlx::query(
            r#"
            UPDATE wallets w
            SET derivation_index = w.derivation_index + 1
            FROM store_payment_methods pm
            WHERE pm.id = $1 AND w.id = pm.wallet_id
            RETURNING w.derivation_index - 1 AS current_index
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(r.get("current_index")),
            None => Err(RepositoryError::NotFound("payment method not found".into())),
        }
    }
}

impl PgDataService {
    /// Find or create the account wallet holding `xpub` for the store's owner.
    ///
    /// Payment methods are still configured by pasting an xpub, so this is
    /// where that xpub becomes a wallet. Find-or-create, never create blindly:
    /// a second row for a key the account already holds would be a second
    /// counter on it (RCS-234).
    async fn wallet_for_store_xpub(&self, store_id: Uuid, xpub: &str) -> RepositoryResult<Uuid> {
        let row = sqlx::query(
            r#"
            INSERT INTO wallets (user_id, xpub, is_primary)
            SELECT s.owner_id, $2, NOT EXISTS (
                SELECT 1 FROM wallets WHERE user_id = s.owner_id
            )
            FROM stores s WHERE s.id = $1
            ON CONFLICT (user_id, xpub) DO UPDATE SET xpub = EXCLUDED.xpub
            RETURNING id
            "#,
        )
        .bind(store_id)
        .bind(xpub)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        row.map(|r| r.get("id"))
            .ok_or_else(|| RepositoryError::NotFound("store not found".into()))
    }
}
