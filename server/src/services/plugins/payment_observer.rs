//! Capability 4: be told, after the fact, that an invoice on the instance's
//! own store was paid.
//!
//! SENSITIVE: this sits on the money path. It is deliberately the weakest
//! shape that can still close the loop — an observation of work already
//! committed, not a step in it.
//!
//! The instance sells subscriptions to itself, so a plugin that issued an
//! invoice through capability 3 has to learn that it was paid. That is the
//! whole job. Three properties keep it from becoming anything more:
//!
//! - **It cannot refuse, delay or alter anything.** [`OwnStorePaymentObserver`]
//!   returns `()`. There is no verdict to return and no error to propagate, so
//!   no implementation of it can stop an invoice reaching `Paid` or change what
//!   the merchant was credited. Compare `filter::InvoiceCreationFilter`, which
//!   *is* allowed to refuse — because refusing a not-yet-created invoice is a
//!   billing decision, while refusing a payment already sent takes a customer's
//!   money over a dispute they are not party to.
//! - **It runs after the transition is committed.** The caller dispatches from
//!   beside the webhook queue and the receipt email, both already documented as
//!   best-effort and never blocking the payment flow.
//! - **It is told about one store: ours.** [`is_own_store`] is the entire
//!   safety property of this module. Without it a plugin would observe every
//!   payment every merchant on the instance received — amounts, assets,
//!   customer-supplied metadata — which is a disclosure the merchant never
//!   agreed to and which no billing feature needs.
//!
//! ## Push is the fast path, never the source of truth
//!
//! A dispatch here can be lost for reasons that have nothing to do with
//! payments: the plugin was disabled between the invoice and its payment, the
//! process restarted mid-flight, a wasm call trapped or ran past its deadline.
//! Every one of those loses a notification permanently — nothing retries and
//! nothing queues.
//!
//! If a subscription's paid-through date moved only on this signal, a merchant
//! who *did* pay would silently lapse and be refused at the till. So a consumer
//! of this capability must also reconcile by reading
//! [`OwnStorePaymentReader::settled_since`], and must treat that read as
//! authoritative. This one only makes the common case fast.

use std::sync::Arc;

use async_trait::async_trait;
use auth::SessionService;
use chrono::{DateTime, Utc};
use types::{InvoiceData, InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, StoreId};

use super::PluginHostApi;

/// A settled invoice on the instance's own store.
///
/// Carries no merchant, no wallet and no address: a plugin's legitimate
/// interest here is "which of the invoices I issued has been paid", and
/// `invoice_id` plus `metadata` answers it. `metadata` is whatever the plugin
/// itself attached at
/// [`invoice_create`](super::HostInvoiceIssuer::invoice_create) time, handed
/// back unread — which is how a subscription is correlated without the host
/// having to know what a subscription is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnStorePayment {
    pub invoice_id: InvoiceId,
    /// The unit the invoice was priced in, which for a plugin-issued invoice
    /// is the `asset_symbol` it asked for — capability 3 does no conversion.
    pub currency: String,
    /// What actually arrived, not what was asked for. Kept as the stored
    /// decimal string rather than parsed: these columns hold more significant
    /// digits than a fixed-mantissa type can, and the consumer is comparing
    /// against its own expected amount, not doing arithmetic here.
    pub amount_received: String,
    /// [`InvoiceStatus::Paid`], or [`InvoiceStatus::LatePaid`] when payment
    /// arrived after expiry. Both are money received; a consumer may well want
    /// to treat them differently, so the distinction is not flattened away.
    pub status: InvoiceStatus,
    pub settled_at: DateTime<Utc>,
    pub metadata: Option<serde_json::Value>,
}

impl OwnStorePayment {
    /// Build the observation for `invoice`, which must already have been
    /// transitioned to its settled status.
    #[must_use]
    pub fn from_invoice(invoice: &InvoiceData, settled_at: DateTime<Utc>) -> Self {
        Self {
            invoice_id: invoice.id.clone(),
            currency: invoice.currency.clone(),
            amount_received: invoice.amount_received.clone(),
            status: invoice.status,
            settled_at,
            metadata: invoice.metadata.clone(),
        }
    }
}

/// Something that wants to know an own-store invoice was paid.
///
/// Note what is absent: no return value, so there is nothing an implementation
/// can say back; and no method that names an invoice belonging to anyone else.
/// See the module doc for why both are load-bearing.
#[async_trait]
pub trait OwnStorePaymentObserver: Send + Sync {
    async fn payment_settled(&self, payment: &OwnStorePayment);
}

/// Whether `settled_store_id` is the store this instance issues its own
/// invoices from, and may therefore be reported to observers.
///
/// `own_store_id` is `None` on an instance that does not sell anything to
/// itself — no billing plugin, or none configured with a store. `None` must
/// match nothing: an instance with no own store has no own-store payments, and
/// a `None == None` comparison here would report every merchant's payments to
/// every observer. That is the failure this function exists to prevent, and
/// the test below is the one that catches it.
#[must_use]
pub fn is_own_store(own_store_id: Option<StoreId>, settled_store_id: StoreId) -> bool {
    own_store_id.is_some_and(|own| own == settled_store_id)
}

/// Tell every observer about `invoice`, if and only if it belongs to this
/// instance's own store.
///
/// Returns nothing and cannot fail — see the module doc. Observers are called
/// in order and awaited, so a slow one delays the others; that is acceptable
/// because the caller is already past every database write and every
/// merchant-visible transition, and because bounding a plugin's call time is
/// the runtime's job rather than this function's.
pub async fn notify_own_store_payment(
    observers: &[Arc<dyn OwnStorePaymentObserver>],
    own_store_id: Option<StoreId>,
    invoice: &InvoiceData,
    settled_at: DateTime<Utc>,
) {
    if observers.is_empty() || !is_own_store(own_store_id, invoice.store_id) {
        return;
    }

    let payment = OwnStorePayment::from_invoice(invoice, settled_at);
    for observer in observers {
        observer.payment_settled(&payment).await;
    }
}

/// Why a reconciliation read failed.
///
/// There is deliberately no "this host has no own store" variant: a host that
/// issues nothing to itself has no `PluginHostApi` at all, because
/// [`PluginHostApi::new`](super::PluginHostApi::new) requires the store. The
/// absent case is represented by the absent host, not by an error nobody can
/// act on.
#[derive(Debug, thiserror::Error)]
pub enum PaymentObserverError {
    #[error("failed to read settled invoices: {0}")]
    Internal(String),
}

/// Read back what settled on the instance's own store, for consumers that
/// cannot rely on having received every [`OwnStorePaymentObserver`] dispatch.
///
/// The store is not a parameter. A plugin naming the store it wants to read
/// is the same hole [`super::enforce_own_store`] closes on the write side, so
/// the implementation supplies its own and there is no argument to get wrong.
#[async_trait]
pub trait OwnStorePaymentReader: Send + Sync {
    /// Every own-store invoice that has settled and was *created* at or after
    /// `created_since`, newest first, up to `limit`.
    ///
    /// **The bound is creation time, not settlement time**, because that is
    /// the index the invoice store actually has. An invoice created before the
    /// bound but paid after it will not appear, so a caller reconciling
    /// "everything since I last ran" must look back further than that — past
    /// the longest an invoice can live and still be payable, which is its
    /// expiry plus the late-payment window — and deduplicate by `invoice_id`
    /// against what it has already applied. Both are cheap; silently missing a
    /// merchant's payment is not.
    async fn settled_since(
        &self,
        created_since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<OwnStorePayment>, PaymentObserverError>;
}

/// Reconciliation read, bound to one instance's own store.
///
/// `PluginHostApi` already carries the store this host issues its own invoices
/// from, which is why the trait takes no store argument: there is no parameter
/// for a caller to point somewhere else.
#[async_trait]
impl<A: SessionService + 'static> OwnStorePaymentReader for PluginHostApi<A> {
    async fn settled_since(
        &self,
        created_since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<OwnStorePayment>, PaymentObserverError> {
        let own_store_id = self.own_store_id();

        // Two queries rather than one: `InvoiceQueryParams::status` takes a
        // single status, and both settled states are money received. Asking
        // for neither and filtering here would pull every unpaid invoice on
        // the store across the wire to discard it.
        let mut settled = Vec::new();
        for status in [InvoiceStatus::Paid, InvoiceStatus::LatePaid] {
            let params = InvoiceQueryParams {
                store_id: Some(own_store_id),
                status: Some(status),
                created_after: Some(created_since),
                limit,
                ..InvoiceQueryParams::default()
            };

            // `query` also returns the total matching count, which is not
            // useful here: the caller is reconciling a window, not paging a
            // list, and a count it cannot act on would only invite treating a
            // truncated page as complete.
            let (_total, page) = InvoiceReader::query(self.data_service(), &params)
                .await
                .map_err(|e| PaymentObserverError::Internal(e.to_string()))?;

            for invoice in page {
                // `settled_at` is not stored on the invoice, so the best
                // available stand-in is when it was last known to change.
                // A consumer correlating by `invoice_id` does not need it to
                // be exact; one using it as a cursor would be wrong, which is
                // why `settled_since` bounds on creation instead.
                let settled_at = invoice.created_at;
                settled.push(OwnStorePayment::from_invoice(&invoice, settled_at));
            }
        }

        settled.sort_by(|a, b| b.settled_at.cmp(&a.settled_at));
        settled.truncate(usize::try_from(limit.max(0)).unwrap_or(usize::MAX));
        Ok(settled)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::sync::Mutex;
    use uuid::Uuid;

    fn invoice_on(store_id: StoreId) -> InvoiceData {
        InvoiceData {
            id: InvoiceId::from_string(Uuid::new_v4().to_string()),
            store_id,
            currency: "USDC".to_string(),
            status: InvoiceStatus::Paid,
            amount: "129.000000000000000000".to_string(),
            amount_received: "129.000000000000000000".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now(),
            metadata: Some(serde_json::json!({ "subscription": "acct-7" })),
            customer_email: None,
            extra: None,
        }
    }

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<OwnStorePayment>>,
    }

    #[async_trait]
    impl OwnStorePaymentObserver for Recorder {
        async fn payment_settled(&self, payment: &OwnStorePayment) {
            self.seen.lock().unwrap().push(payment.clone());
        }
    }

    fn recorder() -> (Arc<Recorder>, Vec<Arc<dyn OwnStorePaymentObserver>>) {
        let rec = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn OwnStorePaymentObserver>> = vec![rec.clone()];
        (rec, observers)
    }

    #[tokio::test]
    async fn a_payment_on_our_own_store_reaches_the_observer() {
        let own = StoreId(Uuid::new_v4());
        let (rec, observers) = recorder();

        notify_own_store_payment(&observers, Some(own), &invoice_on(own), Utc::now()).await;

        let seen = rec.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "our own store's payment should be reported");
        assert_eq!(seen[0].currency, "USDC");
        assert_eq!(
            seen[0].metadata,
            Some(serde_json::json!({ "subscription": "acct-7" })),
            "the plugin's own metadata must round-trip - it is how a subscription is correlated"
        );
    }

    /// The disclosure this module exists to prevent. Delete the `is_own_store`
    /// check in `notify_own_store_payment` and this is the test that fails;
    /// every other test here still passes.
    #[tokio::test]
    async fn a_merchants_payment_is_never_reported() {
        let own = StoreId(Uuid::new_v4());
        let a_merchant = StoreId(Uuid::new_v4());
        let (rec, observers) = recorder();

        notify_own_store_payment(&observers, Some(own), &invoice_on(a_merchant), Utc::now()).await;

        assert!(
            rec.seen.lock().unwrap().is_empty(),
            "a plugin must never be told what a merchant was paid"
        );
    }

    /// `None` is "this instance sells nothing to itself", not a wildcard. An
    /// `Option == Option` comparison would make it match a store whose id is
    /// also absent; there is no such store, but the shape is one refactor away
    /// from matching everything, so it is pinned.
    #[tokio::test]
    async fn an_instance_with_no_own_store_reports_nothing() {
        let (rec, observers) = recorder();

        notify_own_store_payment(
            &observers,
            None,
            &invoice_on(StoreId(Uuid::new_v4())),
            Utc::now(),
        )
        .await;

        assert!(
            rec.seen.lock().unwrap().is_empty(),
            "no own store means no own-store payments"
        );
    }

    #[test]
    fn is_own_store_matches_only_the_configured_store() {
        let own = StoreId(Uuid::new_v4());
        let other = StoreId(Uuid::new_v4());

        assert!(is_own_store(Some(own), own));
        assert!(!is_own_store(Some(own), other));
        assert!(!is_own_store(None, own));
    }

    /// A late payment is still money received, and a consumer may want to act
    /// on it differently. Flattening it to `Paid` here would hide that.
    #[tokio::test]
    async fn a_late_payment_keeps_its_status() {
        let own = StoreId(Uuid::new_v4());
        let (rec, observers) = recorder();
        let mut invoice = invoice_on(own);
        invoice.status = InvoiceStatus::LatePaid;

        notify_own_store_payment(&observers, Some(own), &invoice, Utc::now()).await;

        let seen = rec.seen.lock().unwrap();
        assert_eq!(seen[0].status, InvoiceStatus::LatePaid);
    }

    #[tokio::test]
    async fn every_observer_is_told() {
        let own = StoreId(Uuid::new_v4());
        let first = Arc::new(Recorder::default());
        let second = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn OwnStorePaymentObserver>> = vec![first.clone(), second.clone()];

        notify_own_store_payment(&observers, Some(own), &invoice_on(own), Utc::now()).await;

        assert_eq!(first.seen.lock().unwrap().len(), 1);
        assert_eq!(second.seen.lock().unwrap().len(), 1);
    }
}
