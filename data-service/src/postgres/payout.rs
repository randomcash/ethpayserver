//! Payout repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use crate::{PayoutReader, PayoutWriter, RepositoryResult, sqlx_to_repo_error};
use types::{PayoutData, PayoutStatus, StoreId};

use super::PgDataService;

fn db_to_payout_status(s: &str) -> PayoutStatus {
    s.parse().unwrap_or(PayoutStatus::Failed)
}

fn try_row_to_payout(row: &sqlx::postgres::PgRow) -> RepositoryResult<PayoutData> {
    let invoice_ids_json: serde_json::Value =
        row.try_get("invoice_ids").map_err(sqlx_to_repo_error)?;
    let invoice_ids: Vec<String> = serde_json::from_value(invoice_ids_json).unwrap_or_default();

    Ok(PayoutData {
        id: row.try_get("id").map_err(sqlx_to_repo_error)?,
        store_id: StoreId(row.try_get("store_id").map_err(sqlx_to_repo_error)?),
        invoice_ids,
        destination_address: row
            .try_get("destination_address")
            .map_err(sqlx_to_repo_error)?,
        chain_id: row
            .try_get::<i64, _>("chain_id")
            .map_err(sqlx_to_repo_error)? as u64,
        asset_type: row.try_get("asset_type").map_err(sqlx_to_repo_error)?,
        asset_symbol: row.try_get("asset_symbol").map_err(sqlx_to_repo_error)?,
        token_address: row.try_get("token_address").map_err(sqlx_to_repo_error)?,
        amount: row.try_get("amount").map_err(sqlx_to_repo_error)?,
        tx_hash: row.try_get("tx_hash").map_err(sqlx_to_repo_error)?,
        status: db_to_payout_status(
            &row.try_get::<String, _>("status")
                .map_err(sqlx_to_repo_error)?,
        ),
        fee_amount: row.try_get("fee_amount").map_err(sqlx_to_repo_error)?,
        error_message: row.try_get("error_message").map_err(sqlx_to_repo_error)?,
        created_at: row.try_get("created_at").map_err(sqlx_to_repo_error)?,
        confirmed_at: row.try_get("confirmed_at").map_err(sqlx_to_repo_error)?,
    })
}

#[async_trait]
impl PayoutReader for PgDataService {
    async fn get_payout(&self, id: Uuid) -> RepositoryResult<Option<PayoutData>> {
        let row = sqlx::query("SELECT * FROM payouts WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(try_row_to_payout(&r)?)),
            None => Ok(None),
        }
    }

    async fn get_payouts_for_store(
        &self,
        store_id: StoreId,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<(i64, Vec<PayoutData>)> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM payouts WHERE store_id = $1")
            .bind(store_id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let rows = sqlx::query(
            "SELECT * FROM payouts WHERE store_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(store_id.0)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        let payouts: Vec<PayoutData> = rows
            .iter()
            .map(try_row_to_payout)
            .collect::<Result<_, _>>()?;
        Ok((count.0, payouts))
    }

    async fn get_active_payouts(&self) -> RepositoryResult<Vec<PayoutData>> {
        let rows = sqlx::query(
            "SELECT * FROM payouts WHERE status IN ('pending', 'broadcasting') ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payout).collect()
    }
}

#[async_trait]
impl PayoutWriter for PgDataService {
    async fn create_payout(&self, payout: &PayoutData) -> RepositoryResult<()> {
        let invoice_ids_json = serde_json::to_value(&payout.invoice_ids).unwrap_or_default();

        sqlx::query(
            "INSERT INTO payouts (id, store_id, invoice_ids, destination_address, chain_id, asset_type, asset_symbol, token_address, amount, tx_hash, status, fee_amount, error_message, created_at, confirmed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(payout.id)
        .bind(payout.store_id.0)
        .bind(&invoice_ids_json)
        .bind(&payout.destination_address)
        .bind(payout.chain_id as i64)
        .bind(&payout.asset_type)
        .bind(&payout.asset_symbol)
        .bind(&payout.token_address)
        .bind(&payout.amount)
        .bind(&payout.tx_hash)
        .bind(payout.status.as_str())
        .bind(&payout.fee_amount)
        .bind(&payout.error_message)
        .bind(payout.created_at)
        .bind(payout.confirmed_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn update_payout_status(
        &self,
        id: Uuid,
        status: PayoutStatus,
        tx_hash: Option<&str>,
        fee_amount: Option<&str>,
        error_message: Option<&str>,
    ) -> RepositoryResult<()> {
        sqlx::query(
            "UPDATE payouts SET status = $2, tx_hash = COALESCE($3, tx_hash), fee_amount = COALESCE($4, fee_amount), error_message = COALESCE($5, error_message) WHERE id = $1",
        )
        .bind(id)
        .bind(status.as_str())
        .bind(tx_hash)
        .bind(fee_amount)
        .bind(error_message)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn confirm_payout(&self, id: Uuid) -> RepositoryResult<()> {
        sqlx::query("UPDATE payouts SET status = 'confirmed', confirmed_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::db_to_payout_status;
    use types::PayoutStatus;

    #[test]
    fn maps_every_stored_status_string() {
        assert_eq!(db_to_payout_status("pending"), PayoutStatus::Pending);
        assert_eq!(
            db_to_payout_status("broadcasting"),
            PayoutStatus::Broadcasting
        );
        assert_eq!(db_to_payout_status("confirmed"), PayoutStatus::Confirmed);
        assert_eq!(db_to_payout_status("failed"), PayoutStatus::Failed);
    }

    /// An unrecognised status must not be read as an in-flight payout: falling
    /// back to `Failed` keeps `get_active_payouts` from re-broadcasting a row
    /// the code cannot interpret.
    #[test]
    fn unknown_status_falls_back_to_failed() {
        assert_eq!(db_to_payout_status("something-else"), PayoutStatus::Failed);
        assert_eq!(db_to_payout_status(""), PayoutStatus::Failed);
    }

    /// `create_payout` writes `status.as_str()`, so what the writer stores has
    /// to be exactly what the reader maps back.
    #[test]
    fn write_then_read_round_trips_each_status() {
        for status in [
            PayoutStatus::Pending,
            PayoutStatus::Broadcasting,
            PayoutStatus::Confirmed,
            PayoutStatus::Failed,
        ] {
            assert_eq!(db_to_payout_status(status.as_str()), status);
        }
    }
}
