//! `invoice_creation_filters` and `PluginInvoiceCreationFilter` - the export
//! consulted before an invoice is created.

use super::*;

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
