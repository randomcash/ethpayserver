//! Store wallet repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use crate::{RepositoryError, RepositoryResult, StoreWallet, StoreWalletReader, StoreWalletWriter, sqlx_to_repo_error};
use super::PgDataService;

fn row_to_wallet(row: &sqlx::postgres::PgRow) -> StoreWallet {
    StoreWallet {
        id: row.get("id"),
        store_id: row.get("store_id"),
        xpub: row.get("xpub"),
        derivation_index: row.get("derivation_index"),
        name: row.get("name"),
        created_at: row.get("created_at"),
    }
}

#[async_trait]
impl StoreWalletReader for PgDataService {
    async fn get_wallet_by_id(&self, wallet_id: Uuid) -> RepositoryResult<Option<StoreWallet>> {
        let row = sqlx::query(
            "SELECT id, store_id, xpub, derivation_index, name, created_at FROM store_wallets WHERE id = $1",
        )
        .bind(wallet_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_wallet))
    }

    async fn get_wallet(&self, store_id: Uuid) -> RepositoryResult<Option<StoreWallet>> {
        let row = sqlx::query(
            "SELECT id, store_id, xpub, derivation_index, name, created_at FROM store_wallets WHERE store_id = $1",
        )
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.as_ref().map(row_to_wallet))
    }
}

#[async_trait]
impl StoreWalletWriter for PgDataService {
    async fn upsert_wallet(
        &self,
        store_id: Uuid,
        xpub: &str,
        name: Option<&str>,
    ) -> RepositoryResult<StoreWallet> {
        let row = sqlx::query(
            r#"
            INSERT INTO store_wallets (store_id, xpub, name)
            VALUES ($1, $2, $3)
            ON CONFLICT (store_id) DO UPDATE SET xpub = $2, name = $3
            RETURNING id, store_id, xpub, derivation_index, name, created_at
            "#,
        )
        .bind(store_id)
        .bind(xpub)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row_to_wallet(&row))
    }

    async fn delete_wallet(&self, store_id: Uuid) -> RepositoryResult<()> {
        let result = sqlx::query("DELETE FROM store_wallets WHERE store_id = $1")
            .bind(store_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound("store wallet not found".into()));
        }

        Ok(())
    }

    async fn next_derivation_index(&self, store_id: Uuid) -> RepositoryResult<i32> {
        let row = sqlx::query(
            r#"
            UPDATE store_wallets
            SET derivation_index = derivation_index + 1
            WHERE store_id = $1
            RETURNING derivation_index - 1 as current_index
            "#,
        )
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(r.get("current_index")),
            None => Err(RepositoryError::NotFound(
                "store wallet not configured".into(),
            )),
        }
    }
}
