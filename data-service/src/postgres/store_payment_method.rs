//! Store payment method repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{RepositoryError, RepositoryResult, sqlx_to_repo_error};
use types::StorePaymentMethod;
use types::{StorePaymentMethodReader, StorePaymentMethodWriter};

fn row_to_payment_method(row: &sqlx::postgres::PgRow) -> StorePaymentMethod {
    let decimals: i16 = row.get("decimals");
    StorePaymentMethod {
        id: row.get("id"),
        store_id: row.get("store_id"),
        chain_id: row.get::<i64, _>("chain_id") as u64,
        token_address: row.get("token_address"),
        asset_symbol: row.get("asset_symbol"),
        decimals: decimals as u8,
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
        let rows = sqlx::query(
            r#"
            SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
            FROM store_payment_methods
            WHERE store_id = $1
            ORDER BY created_at
            "#,
        )
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
        let rows = sqlx::query(
            r#"
            SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
            FROM store_payment_methods
            WHERE store_id = $1 AND enabled = true
            ORDER BY created_at
            "#,
        )
        .bind(store_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_payment_method).collect())
    }

    async fn get_payment_method(&self, id: Uuid) -> RepositoryResult<Option<StorePaymentMethod>> {
        let row = sqlx::query(
            r#"
            SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
            FROM store_payment_methods
            WHERE id = $1
            "#,
        )
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
                sqlx::query(
                    r#"
                    SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
                    FROM store_payment_methods
                    WHERE store_id = $1 AND chain_id = $2 AND token_address = $3
                    "#,
                )
                .bind(store_id)
                .bind(chain_id as i64)
                .bind(addr)
                .fetch_optional(&self.pool)
                .await
            }
            None => {
                sqlx::query(
                    r#"
                    SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
                    FROM store_payment_methods
                    WHERE store_id = $1 AND chain_id = $2 AND token_address IS NULL
                    "#,
                )
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
        let rows = sqlx::query(
            r#"
            SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
            FROM store_payment_methods
            WHERE store_id = $1 AND asset_symbol = $2 AND enabled = true
            ORDER BY created_at
            "#,
        )
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
        let row = sqlx::query(
            r#"
            INSERT INTO store_payment_methods (store_id, chain_id, token_address, asset_symbol, decimals, xpub)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (store_id, chain_id, token_address) DO UPDATE
            SET xpub = $6, asset_symbol = $4, decimals = $5, enabled = true
            RETURNING id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
            "#,
        )
        .bind(store_id)
        .bind(chain_id as i64)
        .bind(token_address)
        .bind(asset_symbol)
        .bind(decimals as i16)
        .bind(xpub)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row_to_payment_method(&row))
    }

    async fn update_payment_method(
        &self,
        id: Uuid,
        enabled: Option<bool>,
        xpub: Option<&str>,
    ) -> RepositoryResult<StorePaymentMethod> {
        // Build dynamic update query
        let row = match (enabled, xpub) {
            (Some(e), Some(x)) => {
                sqlx::query(
                    r#"
                    UPDATE store_payment_methods
                    SET enabled = $2, xpub = $3
                    WHERE id = $1
                    RETURNING id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
                    "#,
                )
                .bind(id)
                .bind(e)
                .bind(x)
                .fetch_optional(&self.pool)
                .await
            }
            (Some(e), None) => {
                sqlx::query(
                    r#"
                    UPDATE store_payment_methods
                    SET enabled = $2
                    WHERE id = $1
                    RETURNING id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
                    "#,
                )
                .bind(id)
                .bind(e)
                .fetch_optional(&self.pool)
                .await
            }
            (None, Some(x)) => {
                sqlx::query(
                    r#"
                    UPDATE store_payment_methods
                    SET xpub = $2
                    WHERE id = $1
                    RETURNING id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
                    "#,
                )
                .bind(id)
                .bind(x)
                .fetch_optional(&self.pool)
                .await
            }
            (None, None) => {
                // No updates, just fetch
                sqlx::query(
                    r#"
                    SELECT id, store_id, chain_id, token_address, asset_symbol, decimals, xpub, derivation_index, enabled, created_at
                    FROM store_payment_methods
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(&self.pool)
                .await
            }
        }
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(row_to_payment_method(&r)),
            None => Err(RepositoryError::NotFound("payment method not found".into())),
        }
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
        let row = sqlx::query(
            r#"
            UPDATE store_payment_methods
            SET derivation_index = derivation_index + 1
            WHERE id = $1
            RETURNING derivation_index - 1 as current_index
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
