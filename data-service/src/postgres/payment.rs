//! Payment repository implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{PaymentReader, PaymentWriter, RepositoryError, RepositoryResult, sqlx_to_repo_error};
use types::{AssetType, InvoiceId, PaymentData, PaymentQueryParams};

use super::{PgDataService, search_contains_pattern, search_prefix_pattern};

/// Bind the payment filter values in exactly the order the WHERE clause names
/// them.
///
/// Extracted so the count query and the data query cannot drift apart. These
/// binds are positional: one list missing a value shifts every later filter
/// onto the wrong placeholder, which does not fail — it silently answers a
/// different question. Two hand-maintained copies of the same sequence is how
/// that happens, so there is now one (RCS-222).
fn bind_payment_filters<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    params: &'q PaymentQueryParams,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    if let Some(store_id) = params.store_id {
        query = query.bind(store_id.0);
    }
    if let Some(ref store_ids) = params.store_ids {
        query = query.bind(store_ids.iter().map(|s| s.0).collect::<Vec<_>>());
    }
    if let Some(ref invoice_id) = params.invoice_id {
        query = query.bind(invoice_id.as_str());
    }
    // Free-text search, in the same position the WHERE clause gives it: the
    // anchored pattern first, then the substring one (RCS-231).
    if let Some(term) = params.search_term() {
        query = query
            .bind(search_prefix_pattern(term))
            .bind(search_contains_pattern(term));
    }
    query
}

/// Convert AssetType to database string.
fn asset_type_to_db(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Native => "native",
        AssetType::ERC20 => "erc20",
    }
}

/// Convert database string to AssetType.
fn db_to_asset_type(s: &str) -> AssetType {
    match s {
        "erc20" => AssetType::ERC20,
        _ => AssetType::Native,
    }
}

/// Common SELECT columns for payment queries
const PAYMENT_SELECT_COLS: &str = r#"
    id, invoice_id, payment_option_id, chain_id, asset_type::text,
    amount::text, asset_symbol, token_address, tx_hash, block_number,
    detected_at, confirmed_at, from_address, reorged, extra,
    credited_amount::text, rate_used::text, rate_applied_at
"#;

#[async_trait]
impl PaymentReader for PgDataService {
    async fn get(&self, id: Uuid) -> RepositoryResult<Option<PaymentData>> {
        let query = format!("SELECT {} FROM payments WHERE id = $1", PAYMENT_SELECT_COLS);
        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(try_row_to_payment(&r)?)),
            None => Ok(None),
        }
    }

    async fn get_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE invoice_id = $1 ORDER BY detected_at DESC",
            PAYMENT_SELECT_COLS
        );
        let rows = sqlx::query(&query)
            .bind(invoice_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn get_awaiting_confirmation(&self) -> RepositoryResult<Vec<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE confirmed_at IS NULL AND reorged = FALSE ORDER BY detected_at ASC",
            PAYMENT_SELECT_COLS
        );
        let rows = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn get_valid_for_invoice(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Vec<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE invoice_id = $1 AND reorged = FALSE ORDER BY detected_at DESC",
            PAYMENT_SELECT_COLS
        );
        let rows = sqlx::query(&query)
            .bind(invoice_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        rows.iter().map(try_row_to_payment).collect()
    }

    async fn has_valid_payments(&self, invoice_id: &InvoiceId) -> RepositoryResult<bool> {
        let row = sqlx::query(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM payments
                WHERE invoice_id = $1 AND reorged = FALSE
            ) as has_payments
            "#,
        )
        .bind(invoice_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.get("has_payments"))
    }

    async fn query(
        &self,
        params: &PaymentQueryParams,
    ) -> RepositoryResult<(i64, Vec<PaymentData>)> {
        // Build dynamic WHERE clause
        let mut conditions = Vec::new();
        let mut bind_idx = 1;
        // Either store filter needs the invoices join - payments carry no
        // store_id of their own (RCS-222).
        let needs_join = params.store_id.is_some() || params.store_ids.is_some();

        if params.store_id.is_some() {
            conditions.push(format!("i.store_id = ${}", bind_idx));
            bind_idx += 1;
        }
        // Membership scoping. An empty list matches nothing, which is the
        // correct answer for a caller who belongs to no store - it must not
        // collapse into "no filter". Binds below repeat this order exactly.
        if params.store_ids.is_some() {
            conditions.push(format!("i.store_id = ANY(${})", bind_idx));
            bind_idx += 1;
        }
        if params.invoice_id.is_some() {
            conditions.push(format!("p.invoice_id = ${}", bind_idx));
            bind_idx += 1;
        }
        // Free-text search (RCS-231). Two binds, each reused by every column
        // that wants that shape: `${bind_idx}` is the anchored `term%` pattern,
        // `${bind_idx + 1}` the `%term%` one.
        //
        // Anchored on `tx_hash` and `invoice_id` because both are identifiers
        // a merchant pastes whole, and because only an anchored pattern can
        // ever be index-served: `%...%` forecloses it for good. Nothing serves
        // it today - `idx_payments_tx_hash` is on the raw column, not
        // `LOWER(...)` - but the query shape is the one a
        // `payments(LOWER(tx_hash) varchar_pattern_ops)` index would satisfy
        // when this gets hot. Substring on `asset_symbol` and `from_address`
        // because neither is indexed at all, so anchoring buys nothing, and a
        // partial match is what someone typing "usd" or a fragment of an
        // address means.
        //
        // Note this touches only `payments` columns, so it needs no join and
        // cannot widen the store scope: the scope conditions above stay ANDed
        // on top (RCS-211, RCS-222).
        if params.search_term().is_some() {
            let matches = [
                format!("LOWER(p.tx_hash) LIKE ${}", bind_idx),
                format!("LOWER(p.invoice_id) LIKE ${}", bind_idx),
                format!("LOWER(p.asset_symbol) LIKE ${}", bind_idx + 1),
                format!("LOWER(p.from_address) LIKE ${}", bind_idx + 1),
            ];
            conditions.push(format!("({})", matches.join(" OR ")));
            bind_idx += 2;
        }
        if let Some(confirmed) = params.confirmed {
            if confirmed {
                conditions.push("p.confirmed_at IS NOT NULL".to_string());
            } else {
                conditions.push("p.confirmed_at IS NULL".to_string());
            }
        }

        let join_clause = if needs_join {
            "JOIN invoices i ON i.id = p.invoice_id"
        } else {
            ""
        };

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Count query
        let count_sql = format!(
            "SELECT COUNT(*) as count FROM payments p {} {}",
            join_clause, where_clause
        );

        // Data query
        let data_sql = format!(
            r#"
            SELECT
                p.id, p.invoice_id, p.payment_option_id, p.chain_id, p.asset_type::text,
                p.amount::text, p.asset_symbol, p.token_address, p.tx_hash, p.block_number,
                p.detected_at, p.confirmed_at, p.from_address, p.reorged, p.extra,
                p.credited_amount::text, p.rate_used::text, p.rate_applied_at
            FROM payments p
            {} {}
            ORDER BY p.detected_at DESC
            LIMIT ${} OFFSET ${}
            "#,
            join_clause,
            where_clause,
            bind_idx,
            bind_idx + 1
        );

        // Bind parameters to count query
        let count_query = bind_payment_filters(sqlx::query(&count_sql), params);

        let count_row = count_query
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;
        let total: i64 = count_row.get("count");

        // Bind parameters to data query
        let data_query = bind_payment_filters(sqlx::query(&data_sql), params)
            .bind(params.limit)
            .bind(params.offset);

        let rows = data_query
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        let payments: Result<Vec<PaymentData>, _> = rows.iter().map(try_row_to_payment).collect();

        Ok((total, payments?))
    }
}

#[async_trait]
impl PaymentWriter for PgDataService {
    async fn upsert(&self, payment: &PaymentData) -> RepositoryResult<()> {
        // Use ON CONFLICT (tx_hash, chain_id) to handle duplicate PaymentDetected events
        // (e.g., after service restart). This is the unique constraint in the DB.
        sqlx::query(
            r#"
            INSERT INTO payments (
                id, invoice_id, payment_option_id, chain_id, asset_type, amount, asset_symbol,
                token_address, tx_hash, block_number, detected_at, confirmed_at, from_address, extra,
                credited_amount, rate_used, rate_applied_at
            ) VALUES (
                $1, $2, $3, $4, $5::asset_type, $6::numeric, $7,
                $8, $9, $10, $11, $12, $13, $14,
                $15::numeric, $16::numeric, $17
            )
            ON CONFLICT (tx_hash, chain_id) DO UPDATE SET
                block_number = COALESCE(EXCLUDED.block_number, payments.block_number),
                confirmed_at = COALESCE(EXCLUDED.confirmed_at, payments.confirmed_at),
                extra = COALESCE(EXCLUDED.extra, payments.extra),
                credited_amount = COALESCE(EXCLUDED.credited_amount, payments.credited_amount),
                rate_used = COALESCE(EXCLUDED.rate_used, payments.rate_used),
                rate_applied_at = COALESCE(EXCLUDED.rate_applied_at, payments.rate_applied_at)
            "#,
        )
        .bind(payment.id)
        .bind(payment.invoice_id.as_str())
        .bind(payment.payment_option_id)
        .bind(payment.chain_id as i64)
        .bind(asset_type_to_db(payment.asset_type))
        .bind(&payment.amount)
        .bind(&payment.asset_symbol)
        .bind(&payment.token_address)
        .bind(&payment.tx_hash)
        .bind(payment.block_number.map(|n| n as i64))
        .bind(payment.detected_at)
        .bind(payment.confirmed_at)
        .bind(&payment.from_address)
        .bind(&payment.extra)
        .bind(&payment.credited_amount)
        .bind(&payment.rate_used)
        .bind(payment.rate_applied_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(())
    }

    async fn mark_confirmed(&self, id: Uuid, confirmed_at: DateTime<Utc>) -> RepositoryResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE payments
            SET confirmed_at = $1
            WHERE id = $2 AND confirmed_at IS NULL
            "#,
        )
        .bind(confirmed_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "Payment not found or already confirmed: {}",
                id
            )));
        }

        Ok(())
    }

    async fn mark_reorged(
        &self,
        invoice_id: &InvoiceId,
        chain_id: u64,
        fork_block: u64,
    ) -> RepositoryResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE payments
            SET reorged = TRUE, confirmed_at = NULL
            WHERE invoice_id = $1
              AND chain_id = $2
              AND block_number >= $3
              AND reorged = FALSE
            "#,
        )
        .bind(invoice_id.as_str())
        .bind(chain_id as i64)
        .bind(fork_block as i64)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(result.rows_affected())
    }
}

// =========================================================================
// Inherent methods (not part of the trait)
// =========================================================================

impl PgDataService {
    /// Look up a payment by chain_id and tx_hash.
    ///
    /// Uses the UNIQUE constraint on (tx_hash, chain_id) for efficient lookup.
    pub async fn get_payment_by_tx_hash(
        &self,
        chain_id: u64,
        tx_hash: &str,
    ) -> RepositoryResult<Option<PaymentData>> {
        let query = format!(
            "SELECT {} FROM payments WHERE chain_id = $1 AND tx_hash = $2",
            PAYMENT_SELECT_COLS
        );
        let row = sqlx::query(&query)
            .bind(chain_id as i64)
            .bind(tx_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlx_to_repo_error)?;

        match row {
            Some(r) => Ok(Some(try_row_to_payment(&r)?)),
            None => Ok(None),
        }
    }
}

/// Convert a database row to PaymentData.
fn try_row_to_payment(row: &sqlx::postgres::PgRow) -> RepositoryResult<PaymentData> {
    let chain_id: i64 = row.get("chain_id");
    let block_number: Option<i64> = row.get("block_number");
    let asset_type_str: String = row.get("asset_type");

    Ok(PaymentData {
        id: row.get("id"),
        invoice_id: InvoiceId::from_string(row.get("invoice_id")),
        payment_option_id: row.get("payment_option_id"),
        chain_id: chain_id as u64,
        asset_type: db_to_asset_type(&asset_type_str),
        amount: row.get("amount"),
        asset_symbol: row.get("asset_symbol"),
        token_address: row.get("token_address"),
        tx_hash: row.get("tx_hash"),
        block_number: block_number.map(|n| n as u64),
        detected_at: row.get("detected_at"),
        confirmed_at: row.get("confirmed_at"),
        from_address: row.get("from_address"),
        reorged: row.get("reorged"),
        extra: row.get("extra"),
        credited_amount: row.get("credited_amount"),
        rate_used: row.get("rate_used"),
        rate_applied_at: row.get("rate_applied_at"),
    })
}

// =============================================================================
// Payment Events
// =============================================================================

use crate::PaymentEventWriter;

#[async_trait]
impl PaymentEventWriter for PgDataService {
    async fn create_event(
        &self,
        invoice_id: &InvoiceId,
        payment_id: Option<Uuid>,
        event_type: &str,
        event_data: Option<serde_json::Value>,
    ) -> RepositoryResult<Uuid> {
        let row = sqlx::query(
            r#"
            INSERT INTO payment_events (invoice_id, payment_id, event_type, event_data)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(invoice_id.as_str())
        .bind(payment_id)
        .bind(event_type)
        .bind(&event_data)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        Ok(row.get("id"))
    }
}

// =============================================================================
// Payment Analytics (RCS-225)
// =============================================================================

use crate::analytics::{PaymentAnalyticsReader, PaymentVolumeBucket, PaymentVolumeQuery};

#[async_trait]
impl PaymentAnalyticsReader for PgDataService {
    async fn payment_volume_by_day(
        &self,
        query: &PaymentVolumeQuery,
    ) -> RepositoryResult<Vec<PaymentVolumeBucket>> {
        // `= ANY('{}')` is already false for every row, but short-circuiting
        // keeps the "no stores means no rows" rule visible in both this
        // implementation and the in-memory double rather than resting on a
        // Postgres detail (RCS-203).
        if query.store_ids.is_empty() {
            return Ok(Vec::new());
        }

        let store_ids: Vec<Uuid> = query.store_ids.iter().map(|s| s.0).collect();

        // `payment_options` is LEFT joined because the FK is ON DELETE SET
        // NULL: a payment outlives the option it was made against, and
        // dropping those rows would understate a merchant's volume. 18 is the
        // same fallback the payment list uses for an unknown asset.
        let rows = sqlx::query(
            r#"
            SELECT
                (p.detected_at AT TIME ZONE 'UTC')::date AS day,
                p.asset_symbol AS asset_symbol,
                COALESCE(po.decimals, 18)::smallint AS decimals,
                SUM(p.amount)::text AS raw_amount,
                COUNT(*) AS payment_count
            FROM payments p
            JOIN invoices i ON i.id = p.invoice_id
            LEFT JOIN payment_options po ON po.id = p.payment_option_id
            WHERE i.store_id = ANY($1)
              AND p.reorged = FALSE
              AND p.detected_at >= $2
              AND p.detected_at < $3
            GROUP BY 1, 2, 3
            ORDER BY 1, 2, 3
            "#,
        )
        .bind(&store_ids)
        .bind(query.since)
        .bind(query.until)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_to_repo_error)?;

        rows.iter()
            .map(|row| {
                let raw_decimals: i16 = row.get("decimals");
                let decimals = u8::try_from(raw_decimals).map_err(|_| {
                    RepositoryError::Database(format!("negative token decimals: {raw_decimals}"))
                })?;
                Ok(PaymentVolumeBucket {
                    day: row.get("day"),
                    asset_symbol: row.get("asset_symbol"),
                    decimals,
                    raw_amount: row
                        .get::<Option<String>, _>("raw_amount")
                        .unwrap_or_default(),
                    payment_count: row.get("payment_count"),
                })
            })
            .collect()
    }
}
