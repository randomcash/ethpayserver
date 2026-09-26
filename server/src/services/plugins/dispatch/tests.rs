#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use payserver_plugin_api::Manifest;
use payserver_plugin_host::host_version;
use std::time::Duration;
use types::StoreId;
use uuid::Uuid;

fn manifest(id: &str, kind: &str, failure_mode: Option<&str>) -> Manifest {
    let mut toml = format!(
        "id = \"{id}\"\nversion = \"0.1.0\"\ndependencies = [\"ethpayserver:^0.1\"]\nkind = \"{kind}\"\nui_schema = 1\n"
    );
    if let Some(mode) = failure_mode {
        toml.push_str(&format!("failure_mode = \"{mode}\"\n"));
    }
    toml.parse().unwrap()
}

fn host() -> Arc<PluginHost> {
    Arc::new(PluginHost::new(
        host_version(),
        3,
        Duration::from_millis(200),
    ))
}

/// A plugin exporting the host ABI (`memory`, `alloc`) plus one hook,
/// whose hook body is supplied. Enough to register, which is what makes
/// `kind` answerable.
fn hook_module(export: &str, body: &str) -> Vec<u8> {
    let text = format!(
        r#"
        (module
            (memory (export "memory") 1)
            (global $next (mut i32) (i32.const 1024))

            (func (export "alloc") (param $len i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $next))
                (global.set $next (i32.add (global.get $next) (local.get $len)))
                (local.get $ptr))

            (func (export "{export}") (param $ptr i32) (param $len i32) (result i64)
                {body})
        )
        "#
    );
    wat::parse_str(&text).unwrap()
}

/// Registers, and traps on any call - the "could not run" path.
fn trapping(export: &str) -> Vec<u8> {
    hook_module(export, "unreachable")
}

/// Answers with `json`, verbatim, from a data segment. This is the only
/// fixture that exercises the wire contract in the direction a real plugin
/// uses it.
fn answering(export: &str, json: &str) -> Vec<u8> {
    let escaped = json.replace('"', "\\\"");
    let len = json.len();
    let text = format!(
        r#"
        (module
            (memory (export "memory") 1)
            (data (i32.const 0) "{escaped}")
            (global $next (mut i32) (i32.const 1024))

            (func (export "alloc") (param $len i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $next))
                (global.set $next (i32.add (global.get $next) (local.get $len)))
                (local.get $ptr))

            (func (export "{export}") (param $ptr i32) (param $len i32) (result i64)
                (i64.or
                    (i64.shl (i64.extend_i32_u (i32.const 0)) (i64.const 32))
                    (i64.extend_i32_u (i32.const {len}))))
        )
        "#
    );
    wat::parse_str(&text).unwrap()
}

fn a_store() -> InvoiceCreationFilterRequest {
    InvoiceCreationFilterRequest {
        store_id: StoreId(Uuid::new_v4()),
        account_id: auth::UserId(Uuid::new_v4()),
    }
}

/// The hazard the kind check exists for. An action plugin that fails
/// closed, registered as a filter, refuses every invoice on the instance.
#[test]
fn only_filters_are_registered_as_filters() {
    let host = host();
    host.register(
        manifest("cash.random.anaction", "action", None),
        &trapping(PAYMENT_SETTLED),
    )
    .unwrap();
    host.register(
        manifest("cash.random.afilter", "filter", None),
        &trapping(FILTER_INVOICE_CREATION),
    )
    .unwrap();

    let loaded = vec![
        PluginId::new("cash.random.anaction").unwrap(),
        PluginId::new("cash.random.afilter").unwrap(),
    ];

    assert_eq!(
        invoice_creation_filters(&host, &loaded).len(),
        1,
        "an action plugin must not be registered as a filter"
    );
}

/// Observers are not kind-filtered, so an action plugin is offered the
/// export. See the function doc for why the asymmetry is safe.
#[test]
fn every_loaded_plugin_is_offered_payment_notifications() {
    let host = host();
    host.register(
        manifest("cash.random.anaction", "action", None),
        &trapping(PAYMENT_SETTLED),
    )
    .unwrap();

    let loaded = vec![PluginId::new("cash.random.anaction").unwrap()];
    assert_eq!(payment_observers(&host, &loaded).len(), 1);
}

/// A plugin id the host has never seen must not become a filter. Without
/// the `is_some_and`, an unknown id would fall through to a default.
#[test]
fn an_unregistered_plugin_is_not_a_filter() {
    let host = host();
    let loaded = vec![PluginId::new("cash.random.ghost").unwrap()];

    assert!(invoice_creation_filters(&host, &loaded).is_empty());
}

/// A filter whose manifest fails closed and whose call cannot run must
/// refuse - but with a message the merchant can read, not the trap text.
#[tokio::test]
async fn a_failed_closed_filter_refuses_without_leaking_internals() {
    let host = host();
    host.register(
        manifest("cash.random.strict", "filter", None),
        &trapping(FILTER_INVOICE_CREATION),
    )
    .unwrap();

    let filter = PluginInvoiceCreationFilter::new(
        Arc::clone(&host),
        PluginId::new("cash.random.strict").unwrap(),
    );

    let verdict = filter.filter_invoice_creation(a_store()).await;

    match verdict {
        FilterVerdict::Deny { reason } => {
            assert_eq!(reason, UNAVAILABLE);
            assert!(
                !reason.contains("export") && !reason.contains("wasm"),
                "a merchant must not be shown our internals: {reason}"
            );
        }
        FilterVerdict::Allow => panic!("a closed filter that could not run must refuse"),
    }
}

/// The same failure on a filter that declared `failure_mode = "open"`
/// allows instead. Billing declares open for exactly this reason: our
/// outage must not become every merchant's outage.
#[tokio::test]
async fn a_failed_open_filter_allows() {
    let host = host();
    host.register(
        manifest("cash.random.lenient", "filter", Some("open")),
        &trapping(FILTER_INVOICE_CREATION),
    )
    .unwrap();

    let filter = PluginInvoiceCreationFilter::new(
        Arc::clone(&host),
        PluginId::new("cash.random.lenient").unwrap(),
    );

    let verdict = filter.filter_invoice_creation(a_store()).await;

    assert_eq!(verdict, FilterVerdict::Allow);
}

/// Reporting needs both halves. The `None` store case is the dangerous
/// one: anything that substituted a default there would hand a plugin
/// some merchant's store.
#[test]
fn own_store_reporting_needs_both_a_store_and_an_observer() {
    let host = host();
    host.register(
        manifest("cash.random.obs", "action", None),
        &trapping(PAYMENT_SETTLED),
    )
    .unwrap();
    let observers = payment_observers(&host, &[PluginId::new("cash.random.obs").unwrap()]);
    let store = StoreId(Uuid::new_v4());

    assert!(
        own_store_payment_reporting(Some(store), observers.clone()).is_some(),
        "a store and an observer is the one combination that reports"
    );
    assert!(
        own_store_payment_reporting(None, observers).is_none(),
        "observers with no configured store must never be given one"
    );
    assert!(
        own_store_payment_reporting(Some(store), Vec::new()).is_none(),
        "a store with nothing to notify reports nothing"
    );
}

/// The wire contract in the direction a real plugin uses it: the plugin
/// answers, and its own words reach the merchant unchanged.
///
/// The refusal message is the plugin's one chance to say what to do about
/// it ("your subscription lapsed"), so replacing it with our own generic
/// text would make every billing refusal unactionable. Every other filter
/// test here exercises a path where the plugin never answered at all.
#[tokio::test]
async fn a_plugins_refusal_reaches_the_merchant_verbatim() {
    let host = host();
    host.register(
        manifest("cash.random.answers", "filter", None),
        &answering(
            FILTER_INVOICE_CREATION,
            r#"{"allow":false,"reason":"Your subscription lapsed on 1 September."}"#,
        ),
    )
    .unwrap();

    let filter = PluginInvoiceCreationFilter::new(
        Arc::clone(&host),
        PluginId::new("cash.random.answers").unwrap(),
    );

    match filter.filter_invoice_creation(a_store()).await {
        FilterVerdict::Deny { reason } => {
            assert_eq!(reason, "Your subscription lapsed on 1 September.");
        }
        FilterVerdict::Allow => panic!("the plugin refused; the host allowed"),
    }
}

/// The same path, answering the other way. Without this, a filter stuck on
/// "deny" would pass every other test in this module.
#[tokio::test]
async fn a_plugin_that_allows_is_not_overridden() {
    let host = host();
    host.register(
        manifest("cash.random.permits", "filter", None),
        &answering(FILTER_INVOICE_CREATION, r#"{"allow":true}"#),
    )
    .unwrap();

    let filter = PluginInvoiceCreationFilter::new(
        Arc::clone(&host),
        PluginId::new("cash.random.permits").unwrap(),
    );

    assert_eq!(
        filter.filter_invoice_creation(a_store()).await,
        FilterVerdict::Allow
    );
}

/// Observers are not kind-filtered here either, mirroring payment
/// notifications - see that function's doc for why.
#[test]
fn every_loaded_plugin_is_offered_account_closed() {
    let host = host();
    host.register(
        manifest("cash.random.anaction", "action", None),
        &trapping(ACCOUNT_CLOSED),
    )
    .unwrap();

    let loaded = vec![PluginId::new("cash.random.anaction").unwrap()];
    assert_eq!(account_closed_observers(&host, &loaded).len(), 1);
}

/// An observer must never panic or block on a plugin that cannot answer -
/// the account is already deleted by the time it is called.
#[tokio::test]
async fn an_account_closed_observer_survives_a_plugin_that_cannot_answer() {
    let host = host();
    host.register(
        manifest("cash.random.broken", "action", None),
        &trapping(ACCOUNT_CLOSED),
    )
    .unwrap();

    let observer = PluginAccountClosedObserver::new(
        Arc::clone(&host),
        PluginId::new("cash.random.broken").unwrap(),
    );

    observer.account_closed(UserId(Uuid::new_v4())).await;
}

/// An observer must never panic or block on a plugin that cannot answer -
/// the payment flow is already committed by the time it is called.
#[tokio::test]
async fn an_observer_survives_a_plugin_that_cannot_answer() {
    let host = host();
    host.register(
        manifest("cash.random.broken", "action", None),
        &trapping(PAYMENT_SETTLED),
    )
    .unwrap();

    let observer = PluginPaymentObserver::new(
        Arc::clone(&host),
        PluginId::new("cash.random.broken").unwrap(),
    );

    observer
        .payment_settled(&OwnStorePayment {
            invoice_id: types::InvoiceId::new(),
            currency: "USDC".to_string(),
            amount_received: "129".to_string(),
            status: types::InvoiceStatus::Paid,
            settled_at: chrono::Utc::now(),
            metadata: None,
        })
        .await;
}

/// The wire contract in the direction a real plugin uses it: the plugin
/// cancels and says so, and the admin who asked gets `Cancelled` back.
#[tokio::test]
async fn a_plugin_that_cancels_reports_cancelled() {
    let host = host();
    host.register(
        manifest("cash.random.billing", "action", None),
        &answering(CANCEL_SUBSCRIPTION, r#"{"cancelled":true}"#),
    )
    .unwrap();

    let outcome = cancel_subscription(
        &host,
        &PluginId::new("cash.random.billing").unwrap(),
        "acct-1",
    )
    .await;

    assert_eq!(outcome, CancelSubscriptionOutcome::Cancelled);
}

/// A plugin that ran but declined - no such account, already cancelled -
/// must not be reported as a success, and its own words must reach the
/// caller, same as a filter's refusal.
#[tokio::test]
async fn a_plugin_that_declines_reports_the_refusal_and_its_reason() {
    let host = host();
    host.register(
        manifest("cash.random.billing", "action", None),
        &answering(
            CANCEL_SUBSCRIPTION,
            r#"{"cancelled":false,"reason":"no such account"}"#,
        ),
    )
    .unwrap();

    let outcome = cancel_subscription(
        &host,
        &PluginId::new("cash.random.billing").unwrap(),
        "acct-does-not-exist",
    )
    .await;

    assert_eq!(
        outcome,
        CancelSubscriptionOutcome::Refused {
            reason: Some("no such account".to_string())
        }
    );
}

/// A plugin that cannot run the export at all - not installed, or
/// installed but trapping - must not be reported as a success either.
/// Unlike a filter, there is no manifest failure mode to fall back to:
/// an admin action that no-ops silently would claim it worked.
#[tokio::test]
async fn a_plugin_that_cannot_run_reports_could_not_run() {
    let host = host();
    host.register(
        manifest("cash.random.billing", "action", None),
        &trapping(CANCEL_SUBSCRIPTION),
    )
    .unwrap();

    let outcome = cancel_subscription(
        &host,
        &PluginId::new("cash.random.billing").unwrap(),
        "acct-1",
    )
    .await;

    assert!(matches!(
        outcome,
        CancelSubscriptionOutcome::CouldNotRun { .. }
    ));

    let missing = cancel_subscription(
        &host,
        &PluginId::new("cash.random.ghost").unwrap(),
        "acct-1",
    )
    .await;

    assert!(matches!(
        missing,
        CancelSubscriptionOutcome::CouldNotRun { .. }
    ));
}
