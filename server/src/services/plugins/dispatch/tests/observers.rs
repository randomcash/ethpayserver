//! `payment_observers` and `account_closed_observers` - the two
//! broadcast-to-every-loaded-plugin notifications, and the adapters that
//! carry them.

use super::*;

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
