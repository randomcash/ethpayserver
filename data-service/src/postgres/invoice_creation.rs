//! The transactional half of [`crate::invoice_creation`].
//!
//! Each statement lives in a function taking an executor rather than `&self`,
//! so the same SQL serves both the standalone trait methods (against the pool)
//! and the atomic path (against a transaction). Duplicating it would mean two
//! copies drifting apart, which is how the in-memory double and the real store
//! disagreed once already.

use async_trait::async_trait;
use sqlx::PgConnection;
use types::{InvoiceData, PaymentOptionData, RepositoryResult};

use crate::invoice_creation::InvoiceCreationWriter;
use crate::sqlx_to_repo_error;

use super::PgDataService;
use super::conversions::status_to_db;

/// Insert the invoice row.
pub(super) async fn insert_invoice(
    conn: &mut PgConnection,
    invoice: &InvoiceData,
) -> RepositoryResult<()> {
    sqlx::query(
        r#"
        INSERT INTO invoices (
            id, store_id, currency, status, amount, amount_received,
            created_at, expires_at, metadata, customer_email, extra
        ) VALUES (
            $1, $2, $3, $4::invoice_status, $5::numeric, $6::numeric,
            $7, $8, $9, $10, $11
        )
        "#,
    )
    .bind(invoice.id.as_str())
    .bind(invoice.store_id.0)
    .bind(&invoice.currency)
    .bind(status_to_db(invoice.status))
    .bind(&invoice.amount)
    .bind(&invoice.amount_received)
    .bind(invoice.created_at)
    .bind(invoice.expires_at)
    .bind(&invoice.metadata)
    .bind(&invoice.customer_email)
    .bind(&invoice.extra)
    .execute(&mut *conn)
    .await
    .map_err(sqlx_to_repo_error)?;
    Ok(())
}

/// Insert one payment option.
pub(super) async fn insert_payment_option(
    conn: &mut PgConnection,
    option: &PaymentOptionData,
) -> RepositoryResult<()> {
    // NULL token_address means the native asset; anything else is an ERC-20.
    let asset_type = if option.token_address.is_some() {
        "erc20"
    } else {
        "native"
    };

    sqlx::query(
        r#"
        INSERT INTO payment_options (
            id, invoice_id, payment_method_id, chain_id, asset_type,
            asset_symbol, token_address, decimals, payment_address,
            wallet_id, derivation_index, amount, rate, rate_at, is_active,
            created_at
        ) VALUES (
            $1, $2, $3, $4, $5::asset_type, $6, $7, $8, $9, $10, $11,
            $12::numeric, $13::numeric, $14, $15, $16
        )
        "#,
    )
    .bind(option.id.0)
    .bind(option.invoice_id.as_str())
    .bind(&option.payment_method_id.0)
    .bind(option.chain_id.as_str())
    .bind(asset_type)
    .bind(&option.asset_symbol)
    .bind(&option.token_address)
    .bind(i16::from(option.decimals))
    .bind(&option.payment_address)
    .bind(option.wallet_id)
    .bind(option.derivation_index)
    .bind(&option.amount)
    .bind(&option.rate)
    .bind(option.rate_at)
    .bind(option.is_active)
    .bind(option.created_at)
    .execute(&mut *conn)
    .await
    .map_err(sqlx_to_repo_error)?;
    Ok(())
}

/// Point a watched address at this payment option.
///
/// `invoice_id` and `expires_at` are passed in rather than re-read from the
/// payment option. The standalone writer looks them up, and falls back to
/// "24 hours from now" when the lookup finds nothing - a guess that is wrong
/// whenever the invoice's real expiry differs, and that only never fires today
/// because the option is committed before the lookup runs. Inside a transaction
/// there is nothing to look up that the caller does not already hold.
pub(super) async fn upsert_watched_address(
    conn: &mut PgConnection,
    invoice_id: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
    option: &PaymentOptionData,
) -> RepositoryResult<()> {
    match option.token_address.as_deref() {
        Some(token) => {
            sqlx::query(
                r#"
                INSERT INTO watched_addresses (
                    invoice_id, payment_option_id, chain_id, address, token_address,
                    is_active, expires_at, monitor_notified
                ) VALUES ($1, $2, $3, $4, $5, TRUE, $6, FALSE)
                ON CONFLICT (address, chain_id, token_address) DO UPDATE
                SET payment_option_id = $2, is_active = TRUE, expires_at = $6,
                    monitor_notified = FALSE
                "#,
            )
            .bind(invoice_id)
            .bind(option.id.0)
            .bind(option.chain_id.as_str())
            .bind(&option.payment_address)
            .bind(token)
            .bind(expires_at)
            .execute(&mut *conn)
            .await
            .map_err(sqlx_to_repo_error)?;
        }
        None => {
            // The unique index cannot cover native assets: token_address is NULL
            // and NULL is distinct from NULL, so ON CONFLICT never fires. Lock
            // the row and branch by hand instead. `FOR UPDATE` runs in the
            // caller's transaction here, where the standalone writer opened one
            // of its own - which is precisely why that version could not be
            // composed into a larger unit of work.
            let existing = sqlx::query(
                r#"
                SELECT id FROM watched_addresses
                WHERE LOWER(address) = LOWER($1) AND chain_id = $2
                  AND token_address IS NULL
                FOR UPDATE
                "#,
            )
            .bind(&option.payment_address)
            .bind(option.chain_id.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(sqlx_to_repo_error)?;

            if existing.is_some() {
                sqlx::query(
                    r#"
                    UPDATE watched_addresses
                    SET payment_option_id = $1, is_active = TRUE, expires_at = $2,
                        monitor_notified = FALSE
                    WHERE LOWER(address) = LOWER($3) AND chain_id = $4
                      AND token_address IS NULL
                    "#,
                )
                .bind(option.id.0)
                .bind(expires_at)
                .bind(&option.payment_address)
                .bind(option.chain_id.as_str())
                .execute(&mut *conn)
                .await
                .map_err(sqlx_to_repo_error)?;
            } else {
                sqlx::query(
                    r#"
                    INSERT INTO watched_addresses (
                        invoice_id, payment_option_id, chain_id, address,
                        is_active, expires_at, monitor_notified
                    ) VALUES ($1, $2, $3, $4, TRUE, $5, FALSE)
                    "#,
                )
                .bind(invoice_id)
                .bind(option.id.0)
                .bind(option.chain_id.as_str())
                .bind(&option.payment_address)
                .bind(expires_at)
                .execute(&mut *conn)
                .await
                .map_err(sqlx_to_repo_error)?;
            }
        }
    }
    Ok(())
}

#[async_trait]
impl InvoiceCreationWriter for PgDataService {
    async fn create_invoice_with_options(
        &self,
        invoice: &InvoiceData,
        options: &[PaymentOptionData],
    ) -> RepositoryResult<()> {
        let mut tx = self.pool.begin().await.map_err(sqlx_to_repo_error)?;

        insert_invoice(&mut tx, invoice).await?;
        for option in options {
            insert_payment_option(&mut tx, option).await?;
            upsert_watched_address(&mut tx, invoice.id.as_str(), invoice.expires_at, option)
                .await?;
        }

        // Dropping `tx` without this rolls everything back, which is what every
        // `?` above relies on.
        tx.commit().await.map_err(sqlx_to_repo_error)?;
        Ok(())
    }
}
