//! `MerchantDirectoryReader` against the real `users`/`stores` tables.

use async_trait::async_trait;
use auth::UserId;
use sqlx::Row;
use types::{RepositoryResult, StoreId};

use crate::merchant_directory::{MerchantAccount, MerchantDirectoryReader, MerchantStore};
use crate::sqlx_to_repo_error;

use super::PgDataService;

#[async_trait]
impl MerchantDirectoryReader for PgDataService {
    async fn list_accounts(
        &self,
        offset: i64,
        limit: i64,
    ) -> RepositoryResult<Vec<MerchantAccount>> {
        let rows =
            sqlx::query("SELECT id, created_at FROM users ORDER BY created_at LIMIT $1 OFFSET $2")
                .bind(limit)
                .bind(offset)
                .fetch_all(self.pool())
                .await
                .map_err(sqlx_to_repo_error)?;

        Ok(rows
            .into_iter()
            .map(|row| MerchantAccount {
                id: UserId(row.get("id")),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    async fn list_stores(&self, offset: i64, limit: i64) -> RepositoryResult<Vec<MerchantStore>> {
        let rows = sqlx::query(
            "SELECT id, name, owner_id, archived FROM stores ORDER BY created_at LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows
            .into_iter()
            .map(|row| MerchantStore {
                id: StoreId(row.get("id")),
                name: row.get("name"),
                owner_id: UserId(row.get("owner_id")),
                archived: row.get("archived"),
            })
            .collect())
    }
}
