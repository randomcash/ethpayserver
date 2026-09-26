//! The dispatch path: what turns a loaded plugin into something the rest of
//! the server actually calls.
//!
//! Every capability module beside this one defines a host-side trait and is
//! tested against a hand-written implementation. That proves the capability
//! behaves and proves nothing about whether plugin code ever runs — and until
//! this module existed, none did. `run_action` and `run_filter` had no callers
//! outside the host's own tests, and `AppState::invoice_creation_filters` was
//! populated in tests and never in the live server.
//!
//! This is the adapter layer that closes that gap: each type here implements a
//! capability trait by calling one wasm export on one plugin.
//!
//! # The wire contract
//!
//! The host defines it; a plugin matches it. Arguments and answers are JSON,
//! because [`PluginHost::run_filter`] and [`PluginHost::run_action`] are
//! generic over `serde` and the plugin ABI carries opaque bytes. The exports
//! are named by the constants below rather than inline strings, so the two
//! sides can be compared in one place.
//!
//! # Routing by declared kind
//!
//! A plugin is only offered an export its manifest says it can answer. That is
//! not tidiness: a filter that cannot run resolves to its manifest's
//! [`FailureMode`](payserver_plugin_api::FailureMode), which defaults to
//! *closed*. Register one action plugin as a filter and every invoice on the
//! instance is refused, for as long as it stays installed. [`PluginHost::kind`]
//! is what prevents that, and `only_filters` is the test that pins it.
//!
//! `cancel_subscription` below does not go through that gate, and the reason
//! is what it is dispatched *for*: the filter and payment-observer calls above
//! are broadcast to every loaded plugin of the matching kind on every invoice
//! or every settlement, so a wrong one wedges the instance until someone
//! notices. A cancellation is the opposite shape - one admin, asking one
//! plugin they picked by id, once. A plugin that does not implement the
//! export just reports it could not run, the same as a filter that trapped;
//! nothing else on the instance is affected either way.

use std::sync::Arc;

use async_trait::async_trait;
use payserver_plugin_api::{PluginId, PluginKind};
use serde::{Deserialize, Serialize};

use super::filter::{FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest};
use super::payment_observer::{OwnStorePayment, OwnStorePaymentObserver};
use payserver_plugin_host::{FilterOutcome, PluginHost};

/// The export consulted before an invoice is created.
pub const FILTER_INVOICE_CREATION: &str = "filter_invoice_creation";

/// The export told that an own-store invoice settled.
pub const PAYMENT_SETTLED: &str = "payment_settled";

/// Shown to the merchant when a filter refuses but says nothing useful, or
/// cannot run at all and its manifest fails closed.
///
/// A plugin's own `reason` is shown verbatim, so a refusal that carries one
/// uses it. What must never reach a merchant is the *internal* reason a filter
/// could not run — "wasm trap at 0x1f4", a deadline in milliseconds — which
/// names our implementation and tells them nothing they can act on. Those are
/// logged at warn instead.
const UNAVAILABLE: &str = "Invoice creation is temporarily unavailable. Please try again shortly.";

#[derive(Debug, Serialize)]
struct WireFilterRequest {
    store_id: String,
    /// The merchant who owns the store. Billing is per merchant, so this is
    /// the key a subscription is actually looked up by.
    account_id: String,
}

#[derive(Debug, Deserialize)]
struct WireFilterVerdict {
    allow: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct WirePaymentSettled {
    invoice_id: String,
    currency: String,
    amount_received: String,
    status: String,
    settled_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

impl From<&OwnStorePayment> for WirePaymentSettled {
    fn from(payment: &OwnStorePayment) -> Self {
        Self {
            invoice_id: payment.invoice_id.as_str().to_string(),
            currency: payment.currency.clone(),
            amount_received: payment.amount_received.clone(),
            status: payment.status.to_string(),
            settled_at: payment.settled_at.to_rfc3339(),
            metadata: payment.metadata.clone(),
        }
    }
}

/// One plugin, asked whether an invoice may be created.
pub struct PluginInvoiceCreationFilter {
    host: Arc<PluginHost>,
    id: PluginId,
}

impl PluginInvoiceCreationFilter {
    #[must_use]
    pub fn new(host: Arc<PluginHost>, id: PluginId) -> Self {
        Self { host, id }
    }
}

#[async_trait]
impl InvoiceCreationFilter for PluginInvoiceCreationFilter {
    async fn filter_invoice_creation(
        &self,
        request: InvoiceCreationFilterRequest,
    ) -> FilterVerdict {
        let wire = WireFilterRequest {
            store_id: request.store_id.0.to_string(),
            account_id: request.account_id.0.to_string(),
        };

        let outcome: FilterOutcome<WireFilterVerdict> = self
            .host
            .run_filter(&self.id, FILTER_INVOICE_CREATION, &wire)
            .await;

        match outcome {
            FilterOutcome::Ran(verdict) if verdict.allow => FilterVerdict::Allow,
            FilterOutcome::Ran(verdict) => FilterVerdict::Deny {
                // The plugin's own words where it gave any: this is its one
                // chance to tell the merchant what to do about it.
                reason: verdict.reason.unwrap_or_else(|| UNAVAILABLE.to_string()),
            },
            FilterOutcome::CouldNotRun {
                allowed: true,
                reason,
            } => {
                tracing::warn!(
                    plugin = %self.id,
                    %reason,
                    "invoice-creation filter could not run; its manifest fails open, so the invoice is allowed"
                );
                FilterVerdict::Allow
            }
            FilterOutcome::CouldNotRun {
                allowed: false,
                reason,
            } => {
                tracing::warn!(
                    plugin = %self.id,
                    %reason,
                    "invoice-creation filter could not run; its manifest fails closed, so the invoice is refused"
                );
                // Deliberately not `reason`: that string names our internals
                // and the merchant can act on none of it.
                FilterVerdict::Deny {
                    reason: UNAVAILABLE.to_string(),
                }
            }
        }
    }
}

/// One plugin, told that an invoice on the instance's own store settled.
pub struct PluginPaymentObserver {
    host: Arc<PluginHost>,
    id: PluginId,
}

impl PluginPaymentObserver {
    #[must_use]
    pub fn new(host: Arc<PluginHost>, id: PluginId) -> Self {
        Self { host, id }
    }
}

#[async_trait]
impl OwnStorePaymentObserver for PluginPaymentObserver {
    async fn payment_settled(&self, payment: &OwnStorePayment) {
        // `run_action` is fire-and-forget by construction: it spawns, bounds
        // the call by the host's deadline, records success or failure against
        // the plugin, and returns nothing. That is exactly the guarantee this
        // capability promises — an observer cannot delay or fail a payment.
        self.host.run_action(
            &self.id,
            PAYMENT_SETTLED,
            &WirePaymentSettled::from(payment),
        );
    }
}

/// The export asked to cancel one account's subscription now.
pub const CANCEL_SUBSCRIPTION: &str = "cancel_subscription";

#[derive(Debug, Serialize)]
struct WireCancelSubscriptionRequest<'a> {
    account_id: &'a str,
}

/// What a plugin answers a cancellation request with.
#[derive(Debug, Deserialize)]
struct WireCancelSubscriptionAnswer {
    cancelled: bool,
    /// The plugin's own words, same convention as a filter's `reason`: shown
    /// to the admin verbatim when the plugin gives one.
    #[serde(default)]
    reason: Option<String>,
}

/// What asking a plugin to cancel a subscription came back with.
#[derive(Debug, PartialEq, Eq)]
pub enum CancelSubscriptionOutcome {
    /// The plugin cancelled it.
    Cancelled,
    /// The plugin ran and declined - no such account, already cancelled,
    /// whatever `reason` says.
    Refused { reason: Option<String> },
    /// The call itself did not complete: no such plugin, it trapped, it ran
    /// past the deadline, or its answer did not parse.
    CouldNotRun { reason: String },
}

/// Ask `id` to cancel `account_id`'s subscription now.
///
/// Unlike [`run_filter`](PluginHost::run_filter), there is no failure-mode
/// fallback: an admin action that silently no-ops on an unreachable plugin
/// would report success when nothing happened. Every non-success path is
/// returned instead of papered over.
pub async fn cancel_subscription(
    host: &PluginHost,
    id: &PluginId,
    account_id: &str,
) -> CancelSubscriptionOutcome {
    let wire = WireCancelSubscriptionRequest { account_id };
    match host
        .run_query::<_, WireCancelSubscriptionAnswer>(id, CANCEL_SUBSCRIPTION, &wire)
        .await
    {
        Ok(answer) if answer.cancelled => CancelSubscriptionOutcome::Cancelled,
        Ok(answer) => CancelSubscriptionOutcome::Refused {
            reason: answer.reason,
        },
        Err(reason) => CancelSubscriptionOutcome::CouldNotRun { reason },
    }
}

/// Adapters for every loaded plugin whose manifest declares it a filter.
///
/// Plugins that declare any other kind are skipped rather than registered and
/// left to fail — see the module doc on why a mis-registered filter refuses
/// every invoice on the instance.
#[must_use]
pub fn invoice_creation_filters(
    host: &Arc<PluginHost>,
    loaded: &[PluginId],
) -> Vec<Arc<dyn InvoiceCreationFilter>> {
    loaded
        .iter()
        .filter(|id| host.kind(id).is_some_and(PluginKind::is_filter))
        .map(|id| {
            Arc::new(PluginInvoiceCreationFilter::new(
                Arc::clone(host),
                id.clone(),
            )) as Arc<dyn InvoiceCreationFilter>
        })
        .collect()
}

/// Adapters for every loaded plugin, to be told about own-store payments.
///
/// Unfiltered by kind, unlike the filters above, and the asymmetry is
/// deliberate: a plugin that does not export `payment_settled` simply records
/// a failure and nothing else happens, because an action has no failure mode
/// to resolve and no caller waiting on an answer. There is no equivalent of
/// "refuses every invoice" to protect against here.
#[must_use]
pub fn payment_observers(
    host: &Arc<PluginHost>,
    loaded: &[PluginId],
) -> Vec<Arc<dyn OwnStorePaymentObserver>> {
    loaded
        .iter()
        .map(|id| {
            Arc::new(PluginPaymentObserver::new(Arc::clone(host), id.clone()))
                as Arc<dyn OwnStorePaymentObserver>
        })
        .collect()
}

/// Decide whether own-store payment reporting is on, given what the boot
/// found.
///
/// Both halves are required and the asymmetry matters: a configured store with
/// no plugins has nobody to notify, and observers with no configured store must
/// never be handed a guess at which store is ours — that guess is how a plugin
/// ends up reading a merchant's payments.
///
/// Extracted from `main` deliberately. The decision is three lines and lives in
/// a binary nothing can test, which is precisely the shape of the wiring bugs
/// this repo keeps finding: the capability works, the boot never switches it on,
/// and every test still passes.
#[must_use]
pub fn own_store_payment_reporting(
    operator_store_id: Option<types::StoreId>,
    observers: Vec<Arc<dyn OwnStorePaymentObserver>>,
) -> Option<(types::StoreId, Vec<Arc<dyn OwnStorePaymentObserver>>)> {
    if observers.is_empty() {
        return None;
    }
    operator_store_id.map(|store_id| (store_id, observers))
}

#[cfg(test)]
mod tests {
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
}
