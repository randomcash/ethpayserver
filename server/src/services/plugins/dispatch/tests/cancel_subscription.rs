//! `cancel_subscription` - the one-plugin, one-shot admin action, as opposed
//! to the broadcast-to-every-loaded-plugin shape the other dispatch tests
//! cover.

use super::*;

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
