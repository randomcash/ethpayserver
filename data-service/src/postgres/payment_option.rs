//! Payment option repository implementation.

use async_trait::async_trait;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    PaymentOptionReader, PaymentOptionWriter, RepositoryResult, sqlx_to_repo_error,
};
use types::{InvoiceId, PaymentMethodId, PaymentOptionData, PaymentOptionId};

use super::PgDataService;

#[async_trait]
impl PaymentOptionReader for PgDataService {
    async fn get(&self, id: &PaymentOptionId) -> RepositoryResult<Option<PaymentOptionData>> {
        let row = sqlx::query(
            r#"
            SELECT
                id, invoice_id, payment_method_id, chain_id, asset_symbol,
                token_address, decimals, payment_address, amount::text,
                rate::text, rate_at, is_active, created_at
            FROM payment_options
            WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(row_to_payment_option(&r))),
            None => Ok(None),
        }
    }

    async fn get_for_invoice(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Vec<PaymentOptionData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, invoice_id, payment_method_id, chain_id, asset_symbol,
                token_address, decimals, payment_address, amount::text,
                rate::text, rate_at, is_active, created_at
            FROM payment_options
            WHERE invoice_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(invoice_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_payment_option).collect())
    }

    async fn get_by_payment_method(
        &self,
        invoice_id: &InvoiceId,
        payment_method_id: &PaymentMethodId,
    ) -> RepositoryResult<Option<PaymentOptionData>> {
        let row = sqlx::query(
            r#"
            SELECT
                id, invoice_id, payment_method_id, chain_id, asset_symbol,
                token_address, decimals, payment_address, amount::text,
                rate::text, rate_at, is_active, created_at
            FROM payment_options
            WHERE invoice_id = $1 AND payment_method_id = $2
            "#,
        )
        .bind(invoice_id.as_str())
        .bind(&payment_method_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(row_to_payment_option(&r))),
            None => Ok(None),
        }
    }

    async fn get_active_for_invoice(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Vec<PaymentOptionData>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, invoice_id, payment_method_id, chain_id, asset_symbol,
                token_address, decimals, payment_address, amount::text,
                rate::text, rate_at, is_active, created_at
            FROM payment_options
            WHERE invoice_id = $1 AND is_active = TRUE
            ORDER BY created_at ASC
            "#,
        )
        .bind(invoice_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(rows.iter().map(row_to_payment_option).collect())
    }

    async fn get_by_address(
        &self,
        address: &str,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<Option<PaymentOptionData>> {
        let row = match token_address {
            Some(token) => {
                sqlx::query(
                    r#"
                    SELECT
                        id, invoice_id, payment_method_id, chain_id, asset_symbol,
                        token_address, decimals, payment_address, amount::text,
                        rate::text, rate_at, is_active, created_at
                    FROM payment_options
                    WHERE payment_address = $1 AND chain_id = $2 AND token_address = $3
                    "#,
                )
                .bind(address)
                .bind(chain_id as i64)
                .bind(token)
                .fetch_optional(&self.pool)
                .await
                .map_err(sqlx_to_repo_error)?
            }
            None => {
                sqlx::query(
                    r#"
                    SELECT
                        id, invoice_id, payment_method_id, chain_id, asset_symbol,
                        token_address, decimals, payment_address, amount::text,
                        rate::text, rate_at, is_active, created_at
                    FROM payment_options
                    WHERE payment_address = $1 AND chain_id = $2 AND token_address IS NULL
                    "#,
                )
                .bind(address)
                .bind(chain_id as i64)
                .fetch_optional(&self.pool)
                .await
                .map_err(sqlx_to_repo_error)?
            }
        };

        match row {
            Some(r) => Ok(Some(row_to_payment_option(&r))),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl PaymentOptionWriter for PgDataService {
    async fn create(&self, option: &PaymentOptionData) -> RepositoryResult<()> {
        // Derive asset_type from token_address: NULL = native, NOT NULL = erc20
        let asset_type = if option.token_address.is_some() { "erc20" } else { "native" };

        sqlx::query(
            r#"
            INSERT INTO payment_options (
                id, invoice_id, payment_method_id, chain_id, asset_type,
                asset_symbol, token_address, decimals, payment_address, amount,
                rate, rate_at, is_active, created_at
            ) VALUES (
                $1, $2, $3, $4, $5::asset_type, $6, $7, $8, $9, $10::numeric,
                $11::numeric, $12, $13, $14
            )
            "#,
        )
        .bind(option.id.0)
        .bind(option.invoice_id.as_str())
        .bind(&option.payment_method_id.0)
        .bind(option.chain_id as i64)
        .bind(asset_type)
        .bind(&option.asset_symbol)
        .bind(&option.token_address)
        .bind(option.decimals as i16)
        .bind(&option.payment_address)
        .bind(&option.amount)
        .bind(&option.rate)
        .bind(option.rate_at)
        .bind(option.is_active)
        .bind(option.created_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn update(&self, option: &PaymentOptionData) -> RepositoryResult<()> {
        sqlx::query(
            r#"
            UPDATE payment_options SET
                amount = $1::numeric,
                rate = $2::numeric,
                rate_at = $3,
                is_active = $4
            WHERE id = $5
            "#,
        )
        .bind(&option.amount)
        .bind(&option.rate)
        .bind(option.rate_at)
        .bind(option.is_active)
        .bind(option.id.0)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn deactivate(&self, id: &PaymentOptionId) -> RepositoryResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE payment_options
            SET is_active = FALSE
            WHERE id = $1 AND is_active = TRUE
            "#,
        )
        .bind(id.0)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected() > 0)
    }

    async fn deactivate_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE payment_options
            SET is_active = FALSE
            WHERE invoice_id = $1 AND is_active = TRUE
            "#,
        )
        .bind(invoice_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected())
    }
}

/// Convert a database row to PaymentOptionData.
fn row_to_payment_option(row: &sqlx::postgres::PgRow) -> PaymentOptionData {
    let id: Uuid = row.get("id");
    let chain_id: i64 = row.get("chain_id");
    let decimals: i16 = row.get("decimals");

    PaymentOptionData {
        id: PaymentOptionId(id),
        invoice_id: InvoiceId::from_string(row.get("invoice_id")),
        payment_method_id: PaymentMethodId(row.get("payment_method_id")),
        chain_id: chain_id as u64,
        asset_symbol: row.get("asset_symbol"),
        token_address: row.get("token_address"),
        decimals: decimals as u8,
        payment_address: row.get("payment_address"),
        amount: row.get("amount"),
        rate: row.get("rate"),
        rate_at: row.get("rate_at"),
        is_active: row.get("is_active"),
        created_at: row.get("created_at"),
    }
}
