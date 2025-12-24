//! Store wallet repository implementation.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{RepositoryError, RepositoryResult, sqlx_to_repo_error};
use super::PgDataService;

/// Store wallet configuration for payment address derivation.
#[derive(Debug, Clone)]
pub struct StoreWallet {
    pub id: Uuid,
    pub store_id: Uuid,
    pub xpub: String,
    pub derivation_index: i32,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl PgDataService {
    /// Get wallet configuration for a store.
    pub async fn get_store_wallet(&self, store_id: Uuid) -> RepositoryResult<Option<StoreWallet>> {
        let row = sqlx::query(
            r#"
            SELECT id, store_id, xpub, derivation_index, name, created_at
            FROM store_wallets
            WHERE store_id = $1
            "#,
        )
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(StoreWallet {
                id: r.get("id"),
                store_id: r.get("store_id"),
                xpub: r.get("xpub"),
                derivation_index: r.get("derivation_index"),
                name: r.get("name"),
                created_at: r.get("created_at"),
            })),
            None => Ok(None),
        }
    }

    /// Create or update wallet configuration for a store.
    pub async fn upsert_store_wallet(
        &self,
        store_id: Uuid,
        xpub: &str,
        name: Option<&str>,
    ) -> RepositoryResult<StoreWallet> {
        let row = sqlx::query(
            r#"
            INSERT INTO store_wallets (store_id, xpub, name)
            VALUES ($1, $2, $3)
            ON CONFLICT (store_id)
            DO UPDATE SET xpub = $2, name = $3
            RETURNING id, store_id, xpub, derivation_index, name, created_at
            "#,
        )
        .bind(store_id)
        .bind(xpub)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(StoreWallet {
            id: row.get("id"),
            store_id: row.get("store_id"),
            xpub: row.get("xpub"),
            derivation_index: row.get("derivation_index"),
            name: row.get("name"),
            created_at: row.get("created_at"),
        })
    }

    /// Delete wallet configuration for a store.
    pub async fn delete_store_wallet(&self, store_id: Uuid) -> RepositoryResult<()> {
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

    /// Get and increment the derivation index for a store.
    ///
    /// Returns the current index before incrementing.
    pub async fn get_next_derivation_index(&self, store_id: Uuid) -> RepositoryResult<i32> {
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
