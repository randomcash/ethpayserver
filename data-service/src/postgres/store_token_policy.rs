//! Store token policy repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use super::PgDataService;
use crate::{
    RepositoryResult, StoreTokenPolicyReader, StoreTokenPolicyWithEntries, StoreTokenPolicyWriter,
    TokenPolicyEntryInput, TokenPolicyMode, sqlx_to_repo_error,
};
use types::StoreTokenPolicyEntry;

fn row_to_entry(row: &sqlx::postgres::PgRow) -> StoreTokenPolicyEntry {
    StoreTokenPolicyEntry {
        id: row.get("id"),
        policy_id: row.get("policy_id"),
        chain_id: row.get("chain_id"),
        token_address: row.get("token_address"),
        asset_symbol: row.get("asset_symbol"),
    }
}

#[async_trait]
impl StoreTokenPolicyReader for PgDataService {
    async fn get_token_policy(
        &self,
        store_id: Uuid,
    ) -> RepositoryResult<Option<StoreTokenPolicyWithEntries>> {
        let policy_row = sqlx::query("SELECT * FROM store_token_policies WHERE store_id = $1")
            .bind(store_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let policy_row = match policy_row {
            Some(r) => r,
            None => return Ok(None),
        };

        let policy_id: Uuid = policy_row.get("id");
        let mode_str: String = policy_row.get("mode");
        let mode: TokenPolicyMode = mode_str
            .parse()
            .map_err(|e: String| crate::RepositoryError::InvalidData(e))?;

        let entry_rows =
            sqlx::query("SELECT * FROM store_token_policy_entries WHERE policy_id = $1")
                .bind(policy_id)
                .fetch_all(&self.pool)
                .await
                .map_err(sqlx_to_repo_error)?;

        let entries = entry_rows.iter().map(row_to_entry).collect();

        Ok(Some(StoreTokenPolicyWithEntries {
            id: policy_id,
            store_id: policy_row.get("store_id"),
            mode,
            entries,
            created_at: policy_row.get("created_at"),
            updated_at: policy_row.get("updated_at"),
        }))
    }
}

#[async_trait]
impl StoreTokenPolicyWriter for PgDataService {
    async fn upsert_token_policy(
        &self,
        store_id: Uuid,
        mode: TokenPolicyMode,
        entries: &[TokenPolicyEntryInput],
    ) -> RepositoryResult<StoreTokenPolicyWithEntries> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;

        // Upsert policy header.
        let policy_row = sqlx::query(
            r#"
            INSERT INTO store_token_policies (store_id, mode)
            VALUES ($1, $2)
            ON CONFLICT (store_id) DO UPDATE SET
                mode = EXCLUDED.mode,
                updated_at = NOW()
            RETURNING *
            "#,
        )
        .bind(store_id)
        .bind(mode.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(sqlx_to_repo_error)?;

        let policy_id: Uuid = policy_row.get("id");

        // Replace entries: delete existing, insert new.
        sqlx::query("DELETE FROM store_token_policy_entries WHERE policy_id = $1")
            .bind(policy_id)
            .execute(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;

        let mut result_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let row = sqlx::query(
                r#"
                INSERT INTO store_token_policy_entries (policy_id, chain_id, token_address, asset_symbol)
                VALUES ($1, $2, $3, $4)
                RETURNING *
                "#,
            )
            .bind(policy_id)
            .bind(entry.chain_id)
            .bind(entry.token_address.as_deref())
            .bind(&entry.asset_symbol)
            .fetch_one(&mut *tx)
            .await
            .map_err(sqlx_to_repo_error)?;

            result_entries.push(row_to_entry(&row));
        }

        tx.commit().await.map_err(sqlx_to_repo_error)?;

        Ok(StoreTokenPolicyWithEntries {
            id: policy_id,
            store_id: policy_row.get("store_id"),
            mode,
            entries: result_entries,
            created_at: policy_row.get("created_at"),
            updated_at: policy_row.get("updated_at"),
        })
    }

    async fn delete_token_policy(&self, store_id: Uuid) -> RepositoryResult<()> {
        // Entries cascade-delete via FK.
        sqlx::query("DELETE FROM store_token_policies WHERE store_id = $1")
            .bind(store_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}
