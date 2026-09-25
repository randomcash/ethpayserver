#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn request(json: &str) -> StorageRequest {
    serde_json::from_str(json).unwrap()
}

#[test]
fn a_request_parses_statements_and_parameters() {
    let parsed =
        request(r#"{"statements":[{"sql":"SELECT 1","params":["a",null]},{"sql":"SELECT 2"}]}"#);
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
