//! Settlement tolerance repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use types::InvoiceId;
use uuid::Uuid;

use super::PgDataService;
use crate::{
    RepositoryResult, SettlementAllowance, SettlementToleranceReader, SettlementToleranceWriter,
    sqlx_to_repo_error,
};

#[async_trait]
impl SettlementToleranceReader for PgDataService {
    async fn get_settlement_tolerance(&self, store_id: Uuid) -> RepositoryResult<Option<String>> {
        let row = sqlx::query(
            "SELECT tolerance_percent::text AS tolerance_percent \
             FROM store_settlement_settings WHERE store_id = $1",
        )
        .bind(store_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.map(|r| r.get("tolerance_percent")))
    }

    async fn get_settlement_allowance(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Option<SettlementAllowance>> {
        let row = sqlx::query(
            "SELECT invoice_id, shortfall::text AS shortfall, \
                    tolerance_percent::text AS tolerance_percent, source, recorded_at \
             FROM invoice_settlement_allowances WHERE invoice_id = $1",
        )
        .bind(invoice_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.map(|r| SettlementAllowance {
            invoice_id: r.get("invoice_id"),
            shortfall: r.get("shortfall"),
            tolerance_percent: r.get("tolerance_percent"),
            source: r.get("source"),
            recorded_at: r.get("recorded_at"),
        }))
    }
}

#[async_trait]
impl SettlementToleranceWriter for PgDataService {
    async fn set_settlement_tolerance(
        &self,
        store_id: Uuid,
        tolerance_percent: &str,
    ) -> RepositoryResult<()> {
        sqlx::query(
            "INSERT INTO store_settlement_settings (store_id, tolerance_percent) \
             VALUES ($1, $2::numeric) \
             ON CONFLICT (store_id) DO UPDATE \
             SET tolerance_percent = EXCLUDED.tolerance_percent, updated_at = NOW()",
        )
        .bind(store_id)
        .bind(tolerance_percent)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;
        Ok(())
    }

    async fn clear_settlement_tolerance(&self, store_id: Uuid) -> RepositoryResult<()> {
        sqlx::query("DELETE FROM store_settlement_settings WHERE store_id = $1")
            .bind(store_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;
        Ok(())
    }

    async fn record_settlement_allowance(
        &self,
        invoice_id: &InvoiceId,
        shortfall: &str,
        tolerance_percent: &str,
        source: &str,
    ) -> RepositoryResult<()> {
        sqlx::query(
            "INSERT INTO invoice_settlement_allowances \
                 (invoice_id, shortfall, tolerance_percent, source) \
             VALUES ($1, $2::numeric, $3::numeric, $4) \
             ON CONFLICT (invoice_id) DO NOTHING",
        )
        .bind(invoice_id.as_str())
        .bind(shortfall)
        .bind(tolerance_percent)
        .bind(source)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;
        Ok(())
    }
}
