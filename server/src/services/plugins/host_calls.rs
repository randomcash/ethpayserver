//! What a plugin's host imports actually do.
//!
//! The wasm side of this is in `payserver-plugin-host`: a plugin calls
//! `storage_query` or `invoice_create`, gets a length back, and copies the
//! answer out with `host_take`. This is the other end.
//!
//! Two capabilities, and they are not alike. Storage runs SQL on a
//! connection Postgres authenticated as *that plugin's* role, and the
//! database is what confines it. Invoicing has no such backstop - it writes
//! to the money path - so its confinement is that the plugin cannot name a
//! store. See [`PluginCalls::invoice_create`].
//!
//! # The storage contract
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

/// The issuer a plugin's `invoice_create` reaches, published once the server
/// has one.
///
/// This exists to break a genuine cycle rather than to be clever about
/// initialisation order. An issuer is built around `AppState`; `AppState` is
/// built with the capability implementations that come out of plugin
/// loading; plugin loading is where a plugin is handed its host calls. One
/// of the three has to be late, and this is the one where late is harmless:
/// nothing can call a plugin until the router is serving, and the cell is
/// filled before it does.
///
/// A `OnceLock` rather than a `Mutex` because the write happens once during
/// boot and every read after it is on the money path. It also means the
/// capability cannot be swapped out from under a running plugin - whoever
/// could do that could redirect where invoices are issued.
///
/// Unpublished reads as "this host does not issue invoices", which is the
/// same answer an instance with no billing store gives, and the right one:
/// in both cases there is no store this host would be willing to issue on.
#[derive(Clone, Default)]
pub struct DeferredIssuer(Arc<std::sync::OnceLock<Arc<dyn super::HostInvoiceIssuer>>>);

impl DeferredIssuer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the issuer. Returns whether this call is the one that set it;
    /// a second call changes nothing and says so rather than silently
    /// winning or silently losing.
    pub fn publish(&self, issuer: Arc<dyn super::HostInvoiceIssuer>) -> bool {
        self.0.set(issuer).is_ok()
    }

    fn get(&self) -> Option<&Arc<dyn super::HostInvoiceIssuer>> {
        self.0.get()
    }
}

impl std::fmt::Debug for DeferredIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeferredIssuer")
            .field(&self.get().is_some())
            .finish()
    }
}

/// The same late-binding cell for capability 6.
///
/// Separate from [`DeferredIssuer`] rather than one cell holding both,
/// because the two are available under different conditions: issuing needs
/// the instance's own store and reading a merchant's volume does not. Sharing
/// a cell would make an instance with no billing store silently unable to
/// answer a question it can answer perfectly well.
#[derive(Clone, Default)]
pub struct DeferredVolume(Arc<std::sync::OnceLock<Arc<dyn super::MerchantVolumeReader>>>);

impl DeferredVolume {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the reader. Returns whether this call is the one that set it.
    pub fn publish(&self, reader: Arc<dyn super::MerchantVolumeReader>) -> bool {
        self.0.set(reader).is_ok()
    }

    fn get(&self) -> Option<&Arc<dyn super::MerchantVolumeReader>> {
        self.0.get()
    }
}

impl std::fmt::Debug for DeferredVolume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeferredVolume")
            .field(&self.get().is_some())
            .finish()
    }
}

/// The late-bound host capabilities a plugin's imports resolve through.
///
/// One struct rather than one parameter per cell, because these are threaded
/// from `server.rs` through three layers of boot before they reach a plugin,
/// and a fourth capability should be a field here rather than a fourth
/// argument at every layer. Each cell is still published independently: they
/// become available at different moments and under different conditions.
#[derive(Clone, Default, Debug)]
pub struct DeferredCapabilities {
    /// Capability 3: issuing an invoice on the instance's own store.
    pub issuer: DeferredIssuer,
    /// Capability 6: reading what an account settled over a window.
    pub volume: DeferredVolume,
}

/// One plugin's host imports: its database, and whether it may invoice.
pub struct PluginCalls {
    plugin: PluginId,
    pools: Arc<PluginPools>,
    /// Who issues an invoice when this plugin asks for one, if anything
    /// does.
    ///
    /// Unpublished is the ordinary case and not a degraded one: issuing
    /// requires an own store to issue on, and an instance that has not been
    /// told which store is its own has no honest answer to give. Absent
    /// means the plugin is told invoicing is unavailable, rather than the
    /// host guessing at a store.
    issuer: DeferredIssuer,
    /// Who answers when this plugin asks what an account settled, if
    /// anything does. Unpublished means the plugin is told so, rather than
    /// being handed a zero it would read as "this merchant sold nothing".
    volume: DeferredVolume,
    /// The runtime to drive the async database work on.
    ///
    /// [`PluginHostCalls`] is sync because the runtime calls plugins from
    /// inside `spawn_blocking`, so this runs on a blocking thread and
    /// `block_on` is legal there. Captured at construction rather than looked
    /// up per call, because the blocking thread this ends up on is not itself
    /// inside the runtime and `Handle::current()` would fail there.
    handle: tokio::runtime::Handle,
}

impl PluginCalls {
    /// # Panics
    /// If constructed outside a tokio runtime.
    #[must_use]
    pub fn new(plugin: PluginId, pools: Arc<PluginPools>) -> Self {
        Self {
            plugin,
            pools,
            issuer: DeferredIssuer::default(),
            volume: DeferredVolume::default(),
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

/// What a plugin sends to ask for an invoice.
///
/// Note what is not here: a store. Not an optional one, not one that gets
/// checked - the field does not exist, so a plugin cannot express the
/// request that would have to be refused. `enforce_own_store` still guards
/// the host-side API for ordinary callers; on this path the property holds
/// because there is nothing to enforce it against.
///
/// Every value is a string, for the same reason it is on the storage side:
/// an amount is `NUMERIC(38,18)` and a JSON number is a double.
#[derive(Debug, Deserialize)]
struct InvoiceRequest {
    /// Must match one of the store's own enabled payment methods. There is
    /// no conversion path here - the instance prices its own subscription in
    /// something it already accepts.
    asset_symbol: String,
    amount: String,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    customer_email: Option<String>,
}

/// What the plugin gets back: enough to record the invoice against a
/// subscription and to send the merchant to it.
#[derive(Debug, Serialize)]
struct InvoiceIssued {
    invoice_id: String,
    currency: String,
    amount: String,
    status: String,
    expires_at: String,
    /// Where a merchant pays it. A path rather than a URL: the host does not
    /// reliably know its own external origin, and a plugin that rendered a
    /// wrong absolute URL would send a paying merchant somewhere that is not
    /// this instance.
    checkout_path: String,
}

/// What a plugin quotes volume in when it does not say.
///
/// Named rather than defaulted silently: a plugin that omits the currency is
/// asking for "the usual", and the usual on this instance is the unit its
/// brackets are written in.
const DEFAULT_VOLUME_CURRENCY: &str = "USD";

/// A plugin asking what one account settled.
#[derive(Debug, Deserialize)]
struct VolumeRequest {
    account_id: String,
    /// How far back to sum. Clamped host-side - see
    /// [`MAX_WINDOW_DAYS`](super::MAX_WINDOW_DAYS) - because the plugin names
    /// it.
    window_days: u32,
    #[serde(default)]
    currency: String,
}

/// The answer: one number, and what could not be counted towards it.
#[derive(Debug, Serialize)]
struct VolumeAnswer {
    /// A decimal string. Every value crossing this boundary is text - a JSON
    /// number cannot carry what these columns hold.
    volume: String,
    currency: String,
    /// Assets present in the window that could not be priced, and are
    /// therefore missing from `volume`. Their absence makes the answer an
    /// undercount, which can only under-bill.
    unpriced_assets: Vec<String>,
}

impl PluginCalls {
    /// Point this plugin's host calls at the instance's capabilities, which
    /// may not have been published yet.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: &DeferredCapabilities) -> Self {
        self.issuer = capabilities.issuer.clone();
        self.volume = capabilities.volume.clone();
        self
    }

    fn read_volume(&self, request: &VolumeRequest) -> Result<VolumeAnswer, String> {
        let Some(reader) = self.volume.get().cloned() else {
            return Err("this host does not report merchant volume".to_string());
        };

        // The account is parsed here rather than passed through as text, so
        // an id this instance could never have issued is refused before it
        // reaches a query. The plugin holds the same string the page request
        // handed it, which is a `UserId` rendered - anything else is either a
        // bug in the plugin or a plugin asking about something it made up.
        let account_id = uuid::Uuid::parse_str(&request.account_id)
            .map(types::UserId)
            .map_err(|_| format!("{} is not an account id", request.account_id))?;

        let currency = if request.currency.trim().is_empty() {
            DEFAULT_VOLUME_CURRENCY
        } else {
            request.currency.trim()
        };

        let volume = self.handle.block_on(reader.merchant_volume(
            account_id,
            request.window_days,
            currency,
        ))?;

        Ok(VolumeAnswer {
            volume: volume.volume,
            currency: volume.currency,
            unpriced_assets: volume.unpriced_assets,
        })
    }

    fn issue(&self, request: InvoiceRequest) -> Result<InvoiceIssued, String> {
        let Some(issuer) = self.issuer.get().cloned() else {
            return Err(
                "this host does not issue invoices; no own store is configured for it".to_string(),
            );
        };

        // The store is the host's, taken from the issuer that was built
        // around it. `own_store_id()` is the same value `enforce_own_store`
        // would compare against, so the check it performs is trivially
        // satisfied rather than skipped.
        let store_id = issuer.own_store_id();

        let invoice = self
            .handle
            .block_on(issuer.invoice_create(super::InvoiceCreateRequest {
                store_id,
                asset_symbol: request.asset_symbol,
                amount: request.amount,
                metadata: request.metadata,
                customer_email: request.customer_email,
            }))
            .map_err(|e| e.to_string())?;

        Ok(InvoiceIssued {
            checkout_path: format!("/checkout/{}", invoice.id.0),
            invoice_id: invoice.id.0.to_string(),
            currency: invoice.currency,
            amount: invoice.amount,
            status: format!("{:?}", invoice.status).to_lowercase(),
            expires_at: invoice.expires_at.to_rfc3339(),
        })
    }
}

impl PluginHostCalls for PluginCalls {
    fn invoice_create(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let parsed: InvoiceRequest = serde_json::from_slice(request)
            .map_err(|e| format!("could not read the invoice request: {e}"))?;

        let issued = self.issue(parsed)?;
        serde_json::to_vec(&issued)
            .map_err(|e| format!("could not serialise the invoice answer: {e}"))
    }

    fn merchant_volume(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let parsed: VolumeRequest = serde_json::from_slice(request)
            .map_err(|e| format!("could not read the volume request: {e}"))?;

        let answer = self.read_volume(&parsed)?;
        serde_json::to_vec(&answer)
            .map_err(|e| format!("could not serialise the volume answer: {e}"))
    }

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
        let calls = PluginCalls::new(PluginId::new("cash.random.t").unwrap(), pools);
        let err = calls
            .run(request(r#"{"statements":[]}"#))
            .await
            .unwrap_err();
        assert!(err.contains("at least one statement"), "{err}");
    }

    #[tokio::test]
    async fn too_many_statements_are_refused_before_any_run() {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        let calls = PluginCalls::new(PluginId::new("cash.random.t").unwrap(), pools);
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
        let calls = PluginCalls::new(PluginId::new("cash.random.nopool").unwrap(), pools);
        let err = calls
            .run(request(r#"{"statements":[{"sql":"SELECT 1"}]}"#))
            .await
            .unwrap_err();
        assert!(err.contains("no database access"), "{err}");
    }

    /// A double that answers with a fixed volume and records what it was
    /// asked, so the test can check the request survived the JSON boundary.
    struct FixedVolume {
        asked: std::sync::Mutex<Vec<(types::UserId, u32, String)>>,
    }

    #[async_trait::async_trait]
    impl super::super::MerchantVolumeReader for FixedVolume {
        async fn merchant_volume(
            &self,
            account_id: types::UserId,
            window_days: u32,
            currency: &str,
        ) -> Result<super::super::MerchantVolume, String> {
            self.asked
                .lock()
                .unwrap()
                .push((account_id, window_days, currency.to_string()));
            Ok(super::super::MerchantVolume {
                volume: "12345.67".to_string(),
                currency: currency.to_string(),
                unpriced_assets: vec!["FOO".to_string()],
            })
        }
    }

    fn volume_calls(volume: &DeferredVolume) -> PluginCalls {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        PluginCalls::new(PluginId::new("cash.random.volume").unwrap(), pools).with_capabilities(
            &DeferredCapabilities {
                issuer: DeferredIssuer::default(),
                volume: volume.clone(),
            },
        )
    }

    /// The whole point of the capability, through the boundary a plugin
    /// actually reaches it by: JSON in, JSON out, and the reader published
    /// through the deferred cell rather than held directly.
    #[test]
    fn a_published_volume_reader_answers_a_plugin_in_its_own_units() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let reader = Arc::new(FixedVolume {
            asked: std::sync::Mutex::new(Vec::new()),
        });
        let volume = DeferredVolume::new();
        assert!(volume.publish(reader.clone()));
        let calls = volume_calls(&volume);

        let account = types::UserId::new();
        let answer = PluginHostCalls::merchant_volume(
            &calls,
            format!(
                r#"{{"account_id":"{}","window_days":30,"currency":"USD"}}"#,
                account.0
            )
            .as_bytes(),
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_slice(&answer).unwrap();
        assert_eq!(parsed["volume"], "12345.67");
        assert_eq!(parsed["currency"], "USD");
        assert_eq!(parsed["unpriced_assets"][0], "FOO");

        let asked = reader.asked.lock().unwrap();
        assert_eq!(
            asked[0],
            (account, 30, "USD".to_string()),
            "the reader must see the account, window and currency the plugin asked for"
        );
    }

    /// Absent means absent. A plugin that prices on volume and is handed a
    /// zero would read it as "this merchant sold nothing" and bill them the
    /// bottom bracket forever, which is the one wrong answer that looks
    /// entirely normal.
    #[test]
    fn an_unpublished_volume_reader_is_an_error_and_never_a_zero() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let calls = volume_calls(&DeferredVolume::new());
        let err = PluginHostCalls::merchant_volume(
            &calls,
            format!(
                r#"{{"account_id":"{}","window_days":30}}"#,
                types::UserId::new().0
            )
            .as_bytes(),
        )
        .unwrap_err();

        assert!(err.contains("does not report merchant volume"), "{err}");
    }

    /// The account is parsed before it reaches a query. A plugin holds the
    /// string the host handed it; anything else is a plugin asking about
    /// something it made up.
    #[test]
    fn an_account_id_that_is_not_an_account_id_is_refused() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let reader = Arc::new(FixedVolume {
            asked: std::sync::Mutex::new(Vec::new()),
        });
        let volume = DeferredVolume::new();
        volume.publish(reader.clone());
        let calls = volume_calls(&volume);

        let err = PluginHostCalls::merchant_volume(
            &calls,
            br#"{"account_id":"'; DROP TABLE subscriptions; --","window_days":30}"#,
        )
        .unwrap_err();

        assert!(err.contains("is not an account id"), "{err}");
        assert!(
            reader.asked.lock().unwrap().is_empty(),
            "a request that names no real account must not reach the reader"
        );
    }

    /// A plugin that omits the currency gets the instance's own unit, not an
    /// empty string that would make the answer unreadable.
    #[test]
    fn an_omitted_currency_falls_back_to_the_instance_unit() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        let reader = Arc::new(FixedVolume {
            asked: std::sync::Mutex::new(Vec::new()),
        });
        let volume = DeferredVolume::new();
        volume.publish(reader.clone());
        let calls = volume_calls(&volume);

        PluginHostCalls::merchant_volume(
            &calls,
            format!(
                r#"{{"account_id":"{}","window_days":30,"currency":"  "}}"#,
                types::UserId::new().0
            )
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(reader.asked.lock().unwrap()[0].2, DEFAULT_VOLUME_CURRENCY);
    }

    #[test]
    fn a_malformed_request_is_an_error_not_a_panic() {
        let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let calls =
            rt.block_on(async { PluginCalls::new(PluginId::new("cash.random.t").unwrap(), pools) });
        let err = calls.storage_query(b"not json at all").unwrap_err();
        assert!(err.contains("could not read the storage request"), "{err}");
    }

    /// Provision a role, a pool and a table, and hand back the calls object a
    /// plugin would be given.
    async fn live(
        name: &str,
    ) -> Option<(
        PluginCalls,
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
            PluginCalls::new(plugin.clone(), pools),
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

    /// The module doc's own canonical example - `UPDATE subscriptions SET
    /// ... WHERE account_id = $2` - is never run against a real row
    /// anywhere in this file; only `INSERT` is. That gap matters because
    /// the shape is not academic: a plugin marking an account cancelled,
    /// same as one advancing `paid_until`, is exactly this statement, and
    /// its `rows_affected` is the only signal such a plugin has that the
    /// write actually matched something rather than silently no-op'ing on
    /// a stale or wrong account id.
    ///
    /// It is also the sharp edge the comment on `rows_affected` above
    /// warns about, proven rather than just asserted: an `UPDATE` with no
    /// `RETURNING` reports zero rows *even when it matches and changes
    /// one*, because this call counts rows `fetch_all` returned, not rows
    /// Postgres changed. `RETURNING` is what turns that back into a
    /// trustworthy signal.
    #[tokio::test]
    #[ignore]
    async fn an_update_without_returning_cannot_confirm_its_own_match() {
        let Some((calls, schema, storage, plugin)) = live("cash.random.hc.update").await else {
            return;
        };

        calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"INSERT INTO \"{schema}\".subscriptions (account_id) VALUES ($1)","params":["acct-3"]}}]}}"#
            )))
            .await
            .expect("the fixture row must insert");

        let blind = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"UPDATE \"{schema}\".subscriptions SET paid_until = now() WHERE account_id = $1","params":["acct-3"]}}]}}"#
            )))
            .await
            .expect("the update itself must succeed");
        assert_eq!(
            blind.results[0].rows_affected, "0",
            "documented behaviour: an UPDATE with no RETURNING reports zero \
             rows even though this one matched and changed acct-3"
        );

        let confirmed = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"UPDATE \"{schema}\".subscriptions SET paid_until = now() WHERE account_id = $1 RETURNING account_id","params":["acct-3"]}}]}}"#
            )))
            .await
            .expect("the update itself must succeed");
        assert_eq!(
            confirmed.results[0].rows_affected, "1",
            "RETURNING is what makes rows_affected trustworthy for a write \
             that needs to know whether it matched anything"
        );

        let missed = calls
            .run(request(&format!(
                r#"{{"statements":[{{"sql":"UPDATE \"{schema}\".subscriptions SET paid_until = now() WHERE account_id = $1 RETURNING account_id","params":["acct-does-not-exist"]}}]}}"#
            )))
            .await
            .expect("an update matching nothing is not an error");
        assert_eq!(
            missed.results[0].rows_affected, "0",
            "and a real miss still reads as zero, so the signal round-trips \
             both ways"
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
