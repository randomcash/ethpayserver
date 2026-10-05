use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::{Column, Row};

use crate::services::plugins::pools::PluginPoolError;

use super::PluginCalls;

/// Most rows one call may return, across all its statements.
const MAX_ROWS: usize = 1_000;

/// Most bytes one answer may occupy once serialised.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Most statements one call may carry. A transaction is meant to be a unit of
/// work, not a migration.
const MAX_STATEMENTS: usize = 32;

#[derive(Debug, Deserialize)]
struct StorageRequest {
    statements: Vec<Statement>,
}

#[derive(Debug, Deserialize)]
struct Statement {
    sql: String,
    /// `null` is a genuine SQL NULL, not a missing parameter.
    #[serde(default)]
    params: Vec<Option<String>>,
}

#[derive(Debug, Serialize)]
struct StorageResponse {
    results: Vec<StatementResult>,
}

#[derive(Debug, Serialize)]
struct StatementResult {
    /// A string like every other value, for one rule rather than two.
    rows_affected: String,
    rows: Vec<BTreeMap<String, Option<String>>>,
}

impl PluginCalls {
    pub(super) fn storage_query_impl(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let parsed: StorageRequest = serde_json::from_slice(request)
            .map_err(|e| format!("could not read the storage request: {e}"))?;

        let response = self.handle.block_on(self.run(parsed))?;

        let bytes = serde_json::to_vec(&response)
            .map_err(|e| format!("could not serialise the storage answer: {e}"))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "a storage answer may be at most {MAX_RESPONSE_BYTES} bytes, got {}; \
                 select fewer columns or rows",
                bytes.len()
            ));
        }
        Ok(bytes)
    }

    async fn run(&self, request: StorageRequest) -> Result<StorageResponse, String> {
        if request.statements.is_empty() {
            return Err("a storage call must carry at least one statement".to_string());
        }
        if request.statements.len() > MAX_STATEMENTS {
            return Err(format!(
                "a storage call may carry at most {MAX_STATEMENTS} statements, got {}",
                request.statements.len()
            ));
        }

        let (permit, mut tx) = self.pools.begin(&self.plugin).await.map_err(|e| match e {
            // Worth distinguishing in the message: a plugin that is merely
            // queued behind a busy instance may sensibly retry, and one with
            // no access at all never will.
            PluginPoolError::Busy => "the instance is busy; try again".to_string(),
            other => other.to_string(),
        })?;

        let mut results = Vec::with_capacity(request.statements.len());
        let mut total_rows = 0usize;

        for (index, statement) in request.statements.iter().enumerate() {
            let mut query = sqlx::query(&statement.sql);
            for param in &statement.params {
                query = query.bind(param.clone());
            }

            let rows = query
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| format!("statement {index}: {e}"))?;

            total_rows += rows.len();
            if total_rows > MAX_ROWS {
                // The transaction is dropped unsent, so nothing this call did
                // is kept. A partial answer would be worse than none: the
                // plugin cannot tell it was truncated.
                return Err(format!(
                    "a storage call may return at most {MAX_ROWS} rows; narrow the query"
                ));
            }

            let mut decoded = Vec::with_capacity(rows.len());
            for row in &rows {
                decoded.push(decode_row(row, index)?);
            }

            results.push(StatementResult {
                // `fetch_all` reports rows returned rather than rows changed,
                // so an UPDATE with no RETURNING reports zero. Stated here
                // because "rows_affected: 0" from a successful UPDATE would
                // otherwise look like it matched nothing.
                rows_affected: decoded.len().to_string(),
                rows: decoded,
            });
        }

        // Reached only if every statement succeeded; any `?` above drops the
        // transaction unsent, which rolls it back.
        //
        // Worth knowing for whoever changes this next: committing on the
        // error path instead would *also* roll back, because Postgres aborts
        // a transaction on the first failed statement and treats a subsequent
        // COMMIT as ROLLBACK. So the obvious way to break atomicity here does
        // not actually break it - what would is giving each statement its own
        // transaction, which is what
        // `a_failing_statement_rolls_back_everything_before_it` is written to
        // catch.
        tx.commit()
            .await
            .map_err(|e| format!("could not commit: {e}"))?;
        drop(permit);

        Ok(StorageResponse { results })
    }
}

/// Every column of `row` as `Option<String>`.
///
/// A column that is not text fails with a message naming it, because the
/// alternative - converting it here - is where precision goes to die. See the
/// module doc.
fn decode_row(
    row: &sqlx::postgres::PgRow,
    statement: usize,
) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut out = BTreeMap::new();
    for (index, column) in row.columns().iter().enumerate() {
        let value: Option<String> = row.try_get(index).map_err(|_| {
            format!(
                "statement {statement}: column {:?} is {}, which this interface does not convert. \
                 Cast it in the query - `{}::text` - so the value crosses as the exact text \
                 Postgres renders, rather than through a JSON number that would round it.",
                column.name(),
                column.type_info(),
                column.name()
            )
        })?;
        out.insert(column.name().to_string(), value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
