//! Postgres implementation of the durable event-resume cursor.

use async_trait::async_trait;
use sqlx::Row;
use std::collections::HashMap;

use crate::chain_cursor::{ChainCursor, ChainCursorReader, ChainCursorWriter};
use crate::{RepositoryResult, sqlx_to_repo_error};

use super::PgDataService;

#[async_trait]
impl ChainCursorReader for PgDataService {
    async fn chain_cursors(&self, adapter_id: &str) -> RepositoryResult<HashMap<u64, ChainCursor>> {
        let rows = sqlx::query(
            "SELECT chain_id, epoch, seq, block_height FROM chain_cursors WHERE adapter_id = $1",
        )
        .bind(adapter_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows
            .iter()
            .map(|row| {
                let chain_id: i64 = row.get("chain_id");
                (
                    chain_id as u64,
                    ChainCursor {
                        epoch: row.get("epoch"),
                        seq: row.get("seq"),
                        block_height: row.get("block_height"),
                    },
                )
            })
            .collect())
    }
}

#[async_trait]
impl ChainCursorWriter for PgDataService {
    async fn commit_chain_cursor(
        &self,
        adapter_id: &str,
        chain_id: u64,
        cursor: ChainCursor,
    ) -> RepositoryResult<()> {
        sqlx::query(
            r#"
            INSERT INTO chain_cursors (adapter_id, chain_id, epoch, seq, block_height, updated_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            ON CONFLICT (adapter_id, chain_id) DO UPDATE SET
                epoch = EXCLUDED.epoch,
                seq = EXCLUDED.seq,
                block_height = EXCLUDED.block_height,
                updated_at = NOW()
            "#,
        )
        .bind(adapter_id)
        .bind(chain_id as i64)
        .bind(cursor.epoch)
        .bind(cursor.seq)
        .bind(cursor.block_height)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn reset_chain_watch_notifications(&self, chain_id: u64) -> RepositoryResult<u64> {
        // `watched_addresses.chain_id` is CAIP-2 (see the `caip2` domain
        // migration); the bridge/outbox boundary this method is called from
        // still speaks the raw EIP-155 number, so it is converted here at
        // the boundary rather than pushing that conversion onto every
        // caller.
        let caip2_chain_id = types::ChainId::evm(chain_id);
        let result = sqlx::query(
            "UPDATE watched_addresses SET monitor_notified = FALSE WHERE chain_id = $1 AND is_active = TRUE",
        )
        .bind(caip2_chain_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected())
    }
}
