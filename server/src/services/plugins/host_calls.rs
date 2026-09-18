//! What a plugin's `storage_query` import actually does.
//!
//! The wasm side of this is in `payserver-plugin-host`: a plugin calls
//! `storage_query`, gets a length back, and copies the answer out with
//! `host_take`. This is the other end - the part that runs SQL on a
//! connection Postgres authenticated as *that plugin's* role.
//!
//! # The contract
//!
//! Request:
//!
//! ```json
//! {"statements": [
//!   {"sql": "UPDATE subscriptions SET paid_until = $1::timestamptz WHERE account_id = $2",
//!    "params": ["2026-11-01T00:00:00Z", "acct-7"]}
//! ]}
//! ```
//!
//! Response:
//!
//! ```json
//! {"results": [{"rows_affected": "1", "rows": []}]}
//! ```
//!
//! Four rules, each of which exists for a specific reason:
//!
//! **One call is one transaction.** Every statement in `statements` runs in
//! order inside a single transaction, and the whole thing commits or none of
//! it does. That is not a convenience: billing has to advance `paid_until`
//! *and* record which invoice paid for it, and a crash between those two
//! either credits a merchant twice or loses their payment. A plugin cannot
//! hold a transaction open across calls, because a transaction that outlived
//! a call could hold locks until its deadline with nothing running.
//!
//! **Parameters are bound, never pasted.** Injection is close to moot inside
//! a plugin's own schema - the role cannot reach anything else - but binding
//! keeps one entry to genuinely one statement, since Postgres's extended
//! protocol will not run two.
//!
//! **Every value is a string, both directions.** Parameters arrive as
//! strings and the SQL says what they are (`$1::timestamptz`); results must
//! be text and the SQL says so (`paid_until::text`). The symmetry is not
//! aesthetic. Amounts are `NUMERIC(38,18)`, and a JSON number is an IEEE-754
//! double: `129.000000000000000000` does not survive the round trip. Making
//! every value a string removes the question rather than answering it
//! per-type, and Postgres renders `NUMERIC` to text exactly.
//!
//! A column that is not text is an error naming the column and telling the
//! author to cast it, rather than a silently lossy conversion.
//!
//! Note the shape of that check: it happens per row, so a query that returns
//! **no rows** never exercises it. An author testing against an empty table
//! will see an uncast query succeed and the same query fail once there is
//! data. That is inherent - there is nothing to convert in zero rows - and it
//! is called out here so it reads as a documented edge rather than a bug
//! discovered in production.
//!
//! **The answer is capped.** A plugin can write `SELECT * FROM everything`,
//! and without a limit the host would materialise it, serialise it, and copy
//! it into wasm memory.

use std::collections::BTreeMap;
use std::sync::Arc;

use payserver_plugin_api::PluginId;
use payserver_plugin_host::PluginHostCalls;
use serde::{Deserialize, Serialize};
use sqlx::{Column, Row};

use super::pools::{PluginPoolError, PluginPools};

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

/// One plugin's database access, over its own pool.
pub struct SchemaStorageCalls {
    plugin: PluginId,
    pools: Arc<PluginPools>,
    /// The runtime to drive the async database work on.
    ///
    /// [`PluginHostCalls`] is sync because the runtime calls plugins from
    /// inside `spawn_blocking`, so this runs on a blocking thread and
    /// `block_on` is legal there. Captured at construction rather than looked
    /// up per call, because the blocking thread this ends up on is not itself
    /// inside the runtime and `Handle::current()` would fail there.
    handle: tokio::runtime::Handle,
}

impl SchemaStorageCalls {
    /// # Panics
    /// If constructed outside a tokio runtime.
    #[must_use]
    pub fn new(plugin: PluginId, pools: Arc<PluginPools>) -> Self {
        Self {
            plugin,
            pools,
            handle: tokio::runtime::Handle::current(),
        }
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

impl PluginHostCalls for SchemaStorageCalls {
    fn storage_query(&self, request: &[u8]) -> Result<Vec<u8>, String> {
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn request(json: &str) -> StorageRequest {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_request_parses_statements_and_parameters() {
        let parsed = request(
            r#"{"statements":[{"sql":"SELECT 1","params":["a",null]},{"sql":"SELECT 2"}]}"#,
        );
        assert_eq!(parsed.statements.len(), 2);
        assert_eq!(
            parsed.statements[0].params,
            vec![Some("a".to_string()), None],
            "null must survive as a SQL NULL rather than the string \"null\""
        );
        assert!(parsed.statements[1].params.is_empty(), "params is optional");
    }

    #[test]
    fn an_answer_serialises_every_value_as_a_string() {
        let response = StorageResponse {
            results: vec![StatementResult {
                rows_affected: "1".to_string(),
                rows: vec![BTreeMap::from([
                    (
                        "amount".to_string(),
                        Some("129.000000000000000000".to_string()),
                    ),
                    ("cancelled_at".to_string(), None),
                ])],
            }],
        };
        let json = serde_json::to_string(&response).unwrap();

        assert!(
            json.contains(r#""amount":"129.000000000000000000""#),
            "an amount must cross as a string; a JSON number would round it: {json}"
        );
        assert!(
            json.contains(r#""cancelled_at":null"#),
            "NULL must stay null rather than becoming an empty string: {json}"
        );
        assert!(
            json.contains(r#""rows_affected":"1""#),
            "one rule for every value, counts included: {json}"
        );
    }

    /// The precision claim, stated as a test rather than a comment. A JSON
    /// number cannot carry this value; a JSON string can.
    #[test]
    fn a_numeric_amount_does_not_survive_a_json_number() {
        let exact = "129.000000000000000000";
        let as_number: f64 = exact.parse().unwrap();
        assert_ne!(
            as_number.to_string(),
            exact,
            "if a double round-tripped this exactly, the string rule would be unnecessary"
        );

        let as_string: String = serde_json::from_str(&format!("\"{exact}\"")).unwrap();
        assert_eq!(as_string, exact);
    }

    #[tokio::test]
    async fn an_empty_statement_list_is_refused() {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        let calls = SchemaStorageCalls::new(PluginId::new("cash.random.t").unwrap(), pools);
        let err = calls
            .run(request(r#"{"statements":[]}"#))
            .await
            .unwrap_err();
        assert!(err.contains("at least one statement"), "{err}");
    }

    #[tokio::test]
    async fn too_many_statements_are_refused_before_any_run() {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        let calls = SchemaStorageCalls::new(PluginId::new("cash.random.t").unwrap(), pools);
        let many: Vec<String> = (0..MAX_STATEMENTS + 1)
            .map(|_| r#"{"sql":"SELECT 1"}"#.to_string())
            .collect();
        let err = calls
            .run(request(&format!(
                r#"{{"statements":[{}]}}"#,
                many.join(",")
            )))
            .await
            .unwrap_err();
        assert!(err.contains("at most"), "{err}");
    }

    /// A plugin with no pool must be told so, not silently given the host's
    /// connection.
    #[tokio::test]
    async fn a_plugin_without_a_pool_gets_no_database() {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        let calls = SchemaStorageCalls::new(PluginId::new("cash.random.nopool").unwrap(), pools);
        let err = calls
            .run(request(r#"{"statements":[{"sql":"SELECT 1"}]}"#))
            .await
            .unwrap_err();
        assert!(err.contains("no database access"), "{err}");
    }

    #[test]
    fn a_malformed_request_is_an_error_not_a_panic() {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let calls = rt.block_on(async {
            SchemaStorageCalls::new(PluginId::new("cash.random.t").unwrap(), pools)
        });
        let err = calls.storage_query(b"not json at all").unwrap_err();
        assert!(err.contains("could not read the storage request"), "{err}");
    }

    /// Provision a role, a pool and a table, and hand back the calls object a
    /// plugin would be given.
    async fn live(
        name: &str,
    ) -> Option<(
        SchemaStorageCalls,
        String,
        crate::services::plugins::PluginStorage,
        PluginId,
    )> {
        use crate::services::plugins::{PluginStorage, generate_role_password, role_name};

        let url = std::env::var("DATABASE_URL").ok()?;
        let host_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("DATABASE_URL is set but connecting failed");
        let storage = PluginStorage::new(host_pool.clone());
        let plugin = PluginId::new(name).unwrap();

        let _ = storage.drop_role(&plugin).await;
        let _ = storage.uninstall(&plugin, true).await;

        let migrations = tempfile::tempdir().unwrap();
        storage.install(&plugin, migrations.path()).await.unwrap();
        let password = generate_role_password().unwrap();
        storage.provision_role(&plugin, &password).await.unwrap();

        let schema = role_name(plugin.as_str()).unwrap();
        sqlx::query(&format!(
            "CREATE TABLE \"{schema}\".subscriptions \
             (account_id text primary key, paid_until timestamptz, amount numeric(38,18))"
        ))
        .execute(&host_pool)
        .await
        .unwrap();

        let pools = Arc::new(PluginPools::new(url, 4));
        pools.register(&plugin, &password).await.unwrap();

        Some((
            SchemaStorageCalls::new(plugin.clone(), pools),
            schema,
            storage,
            plugin,
        ))
    }

    /// The whole path, as billing will use it.
    #[tokio::test]
    #[ignore]
    async fn a_plugin_writes_and_reads_its_own_schema() {
        let Some((calls, schema, storage, plugin)) = live("cash.random.hc.roundtrip").await else {
            return;
        };

        let written = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"INSERT INTO \"{schema}\".subscriptions (account_id, paid_until, amount) VALUES ($1, $2::timestamptz, $3::numeric)","params":["acct-7","2026-11-01T00:00:00Z","129.000000000000000000"]}}]}}"#
            )))
            .await
            .expect("a plugin must be able to write its own schema");
        assert_eq!(written.results.len(), 1);

        let read = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"SELECT account_id, amount::text AS amount FROM \"{schema}\".subscriptions WHERE account_id = $1","params":["acct-7"]}}]}}"#
            )))
            .await
            .expect("a plugin must be able to read its own schema");

        let row = &read.results[0].rows[0];
        assert_eq!(row["account_id"], Some("acct-7".to_string()));
        assert_eq!(
            row["amount"],
            Some("129.000000000000000000".to_string()),
            "the amount must cross with every digit Postgres stored"
        );

        storage.drop_role(&plugin).await.unwrap();
        storage.uninstall(&plugin, true).await.unwrap();
    }

    /// The property billing actually depends on: advancing `paid_until` and
    /// recording which invoice paid for it must both happen or neither. A
    /// failure in the second statement must not leave the first committed, or
    /// a merchant is credited for a payment nothing recorded.
    #[tokio::test]
    #[ignore]
    async fn a_failing_statement_rolls_back_everything_before_it() {
        let Some((calls, schema, storage, plugin)) = live("cash.random.hc.atomic").await else {
            return;
        };

        let result = calls
            .run(request(&format!(
                r#"{{"statements":[
                    {{"sql":"INSERT INTO \"{schema}\".subscriptions (account_id) VALUES ($1)","params":["acct-9"]}},
                    {{"sql":"INSERT INTO \"{schema}\".no_such_table (x) VALUES (1)"}}
                ]}}"#
            )))
            .await;
        assert!(result.is_err(), "a bad statement must fail the call");

        let after = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"SELECT count(*)::text AS n FROM \"{schema}\".subscriptions"}}]}}"#
            )))
            .await
            .unwrap();
        assert_eq!(
            after.results[0].rows[0]["n"],
            Some("0".to_string()),
            "the first statement stayed committed; one call is not one transaction"
        );

        storage.drop_role(&plugin).await.unwrap();
        storage.uninstall(&plugin, true).await.unwrap();
    }

    /// A column that is not text must produce an instruction, not a silently
    /// lossy conversion and not an opaque decode failure.
    #[tokio::test]
    #[ignore]
    async fn a_non_text_column_tells_the_author_to_cast_it() {
        let Some((calls, schema, storage, plugin)) = live("cash.random.hc.cast").await else {
            return;
        };

        // A row has to exist: the check runs per row, so an empty result
        // set has nothing to convert and would pass whatever the column type.
        calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"INSERT INTO \"{schema}\".subscriptions (account_id, amount) VALUES ($1, $2::numeric)","params":["acct-1","1.5"]}}]}}"#
            )))
            .await
            .unwrap();

        let err = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"SELECT amount FROM \"{schema}\".subscriptions"}}]}}"#
            )))
            .await
            .expect_err("an uncast numeric column must be refused");

        assert!(
            err.contains("amount"),
            "the message must name the column: {err}"
        );
        assert!(
            err.contains("::text"),
            "the message must say what to do about it: {err}"
        );

        storage.drop_role(&plugin).await.unwrap();
        storage.uninstall(&plugin, true).await.unwrap();
    }

    /// The boundary, at the level a plugin actually reaches it.
    #[tokio::test]
    #[ignore]
    async fn a_plugin_cannot_reach_core_tables_through_a_storage_call() {
        let Some((calls, _schema, storage, plugin)) = live("cash.random.hc.core").await else {
            return;
        };

        let err = calls
            .run(request(
                r#"{"statements":[{"sql":"SELECT id FROM public.installed_plugins LIMIT 1"}]}"#,
            ))
            .await
            .expect_err("a plugin read a core table through storage_query");
        assert!(err.contains("permission denied"), "{err}");

        storage.drop_role(&plugin).await.unwrap();
        storage.uninstall(&plugin, true).await.unwrap();
    }
}
