//! Database type conversions.

use crate::RepositoryError;
use types::InvoiceStatus;

/// Convert Rust InvoiceStatus enum to database string.
///
/// This is infallible since all InvoiceStatus variants are supported.
pub fn status_to_db(status: InvoiceStatus) -> &'static str {
    match status {
        InvoiceStatus::Pending => "pending",
        InvoiceStatus::Processing => "processing",
        InvoiceStatus::PartiallyPaid => "partially_paid",
        InvoiceStatus::LatePaid => "late_paid",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::Expired => "expired",
        InvoiceStatus::Cancelled => "cancelled",
        InvoiceStatus::Refunded => "refunded",
    }
}

/// Convert database string to Rust InvoiceStatus enum.
///
/// Returns an error if the database contains an unknown status value,
/// indicating data corruption or schema mismatch.
pub fn try_db_to_status(s: &str) -> Result<InvoiceStatus, RepositoryError> {
    match s {
        "pending" => Ok(InvoiceStatus::Pending),
        "processing" => Ok(InvoiceStatus::Processing),
        "partially_paid" => Ok(InvoiceStatus::PartiallyPaid),
        "late_paid" => Ok(InvoiceStatus::LatePaid),
        "paid" => Ok(InvoiceStatus::Paid),
        "expired" => Ok(InvoiceStatus::Expired),
        "cancelled" => Ok(InvoiceStatus::Cancelled),
        "refunded" => Ok(InvoiceStatus::Refunded),
        _ => Err(RepositoryError::InvalidData(format!(
            "Unknown invoice status in database: {}",
            s
        ))),
    }
}

/// Read a CAIP-2 chain id from a row.
///
/// Parsing cannot fail against this schema: the column is the `caip2` domain,
/// whose CHECK enforces exactly the grammar `ChainId::parse` enforces. Treated
/// as a schema mismatch rather than made fallible everywhere - `row.get` already
/// panics when a column is not the type its mapper expects, and making every row
/// mapper return `Result` for a case the database forbids would obscure the
/// failures that can actually happen.
pub fn chain_id_from_row(row: &sqlx::postgres::PgRow, column: &str) -> types::ChainId {
    use sqlx::Row;
    let raw: String = row.get(column);
    types::ChainId::parse(raw.as_str()).unwrap_or_else(|e| {
        panic!(
            "column `{column}` holds `{raw}`, which is not a CAIP-2 chain id ({e}). \
             The `caip2` domain should have made this impossible - has the column \
             been altered, or was the caip2 migration skipped?"
        )
    })
}
