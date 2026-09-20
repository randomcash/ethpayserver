//! The pull half of own-store payment reporting.
//!
//! Push is the fast path and never the source of truth. A dispatch is lost
//! whenever the plugin could not take it - disabled after repeated failure,
//! trapped, past its deadline, or simply not loaded because the process was
//! restarting when the payment confirmed. Every one of those is a merchant
//! who paid and was not credited, and none of them is visible: the payment
//! succeeded, the invoice is settled, and only the subscription is wrong.
//!
//! This reads back what actually settled and dispatches it again.
//!
//! # Why replaying is safe
//!
//! Because crediting is idempotent, which is not an accident: the billing
//! plugin's `credited_invoices` table has the invoice id as its primary key,
//! and crediting is one statement whose insert gates the update. A payment
//! delivered twice inserts nothing the second time and advances nothing. The
//! plugin was built that way so this service could exist.
//!
//! # Why the window is generous
//!
//! [`OwnStorePaymentReader::settled_since`] bounds on **creation** time,
//! because that is the index the invoice store has. An invoice created before
//! the bound but paid after it does not appear. So the lookback has to exceed
//! the longest an invoice can live and still be payable - its expiry plus any
//! late-payment window - rather than the interval between runs. Reading the
//! same settled invoice on twenty consecutive passes costs twenty no-op
//! statements; missing one costs a merchant their subscription.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use super::payment_observer::{OwnStorePaymentObserver, OwnStorePaymentReader};

/// How often to reconcile.
///
/// Minutes rather than seconds: this is the safety net under a push path that
/// works, so the cost of being late is a subscription credited a few minutes
/// after the payment rather than a subscription never credited. Seconds would
/// put a plugin call and two invoice queries on a loop for no gain.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How far back each pass looks.
///
/// Deliberately far longer than the interval - see the module doc on why the
/// bound is creation time. A day covers any plausible invoice lifetime plus
/// late payment, and the duplicate reads it causes are free.
pub const DEFAULT_LOOKBACK: Duration = Duration::from_secs(24 * 60 * 60);

/// Most invoices to pull in one pass.
///
/// A ceiling, not a target. An instance that has settled more of its own
/// invoices than this inside the lookback window will not reconcile the
/// oldest of them, which is worth knowing about - so exceeding it is logged
/// rather than silently truncated.
pub const DEFAULT_LIMIT: i64 = 500;

/// Reconcile once: read what settled, and tell every observer again.
///
/// Returns how many payments were re-dispatched, which is the number worth
/// logging - not how many were newly credited, because only the plugin knows
/// that and it deliberately does not say.
pub async fn reconcile_once(
    reader: &dyn OwnStorePaymentReader,
    observers: &[Arc<dyn OwnStorePaymentObserver>],
    lookback: Duration,
    limit: i64,
) -> usize {
    if observers.is_empty() {
        return 0;
    }

    let since = Utc::now()
        - chrono::Duration::from_std(lookback).unwrap_or_else(|_| chrono::Duration::days(1));

    let settled = match reader.settled_since(since, limit).await {
        Ok(settled) => settled,
        Err(e) => {
            // Not fatal and not retried here: the next pass is minutes away
            // and does the same work. Retrying inside one pass would turn a
            // database blip into a tight loop against the same database.
            tracing::warn!(error = %e, "could not read settled own-store payments; will retry");
            return 0;
        }
    };

    if settled.len() as i64 >= limit {
        tracing::warn!(
            limit,
            "reconciliation hit its limit; older settled invoices in the window were not read"
        );
    }

    for payment in &settled {
        for observer in observers {
            observer.payment_settled(payment).await;
        }
    }
    settled.len()
}

/// Run [`reconcile_once`] on a loop, forever.
///
/// Runs once immediately, before the first sleep. The pass that matters most
/// is the one right after a restart - that is precisely when dispatches were
/// missed - and waiting a full interval to make it would leave a merchant who
/// paid during the restart uncredited for that whole time.
pub async fn run(
    reader: Arc<dyn OwnStorePaymentReader>,
    observers: Vec<Arc<dyn OwnStorePaymentObserver>>,
    interval: Duration,
) {
    loop {
        let n = reconcile_once(&*reader, &observers, DEFAULT_LOOKBACK, DEFAULT_LIMIT).await;
        if n > 0 {
            tracing::debug!(payments = n as u64, "reconciled own-store payments");
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};

    use super::super::payment_observer::{OwnStorePayment, PaymentObserverError};
    use super::*;

    fn payment(id: &str) -> OwnStorePayment {
        OwnStorePayment {
            invoice_id: types::InvoiceId::new(),
            currency: "USDC".to_string(),
            amount_received: id.to_string(),
            status: types::InvoiceStatus::Paid,
            settled_at: Utc::now(),
            metadata: None,
        }
    }

    struct Reader {
        settled: Vec<OwnStorePayment>,
        asked_since: Mutex<Option<DateTime<Utc>>>,
        fail: bool,
    }

    #[async_trait]
    impl OwnStorePaymentReader for Reader {
        async fn settled_since(
            &self,
            created_since: DateTime<Utc>,
            _limit: i64,
        ) -> Result<Vec<OwnStorePayment>, PaymentObserverError> {
            *self.asked_since.lock().unwrap() = Some(created_since);
            if self.fail {
                return Err(PaymentObserverError::Internal("database is down".into()));
            }
            Ok(self.settled.clone())
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    #[async_trait]
    impl OwnStorePaymentObserver for Recorder {
        async fn payment_settled(&self, payment: &OwnStorePayment) {
            self.0.lock().unwrap().push(payment.amount_received.clone());
        }
    }

    fn reader(settled: Vec<OwnStorePayment>, fail: bool) -> Reader {
        Reader {
            settled,
            asked_since: Mutex::new(None),
            fail,
        }
    }

    /// The whole point: a payment whose push dispatch was lost is delivered
    /// again. Without this, a plugin that was disabled, trapped or simply not
    /// loaded when the payment confirmed leaves a merchant who paid
    /// uncredited, permanently and invisibly.
    #[tokio::test]
    async fn settled_payments_are_dispatched_again() {
        let recorder = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn OwnStorePaymentObserver>> = vec![recorder.clone()];

        let n = reconcile_once(
            &reader(vec![payment("a"), payment("b")], false),
            &observers,
            DEFAULT_LOOKBACK,
            DEFAULT_LIMIT,
        )
        .await;

        assert_eq!(n, 2);
        assert_eq!(*recorder.0.lock().unwrap(), vec!["a", "b"]);
    }

    /// The lookback must exceed the interval by a wide margin, because the
    /// read is bounded on *creation* time: an invoice created before the
    /// bound and paid after it never appears. A window equal to the interval
    /// would miss exactly the payments this exists to catch.
    #[tokio::test]
    async fn the_window_looks_back_much_further_than_one_interval() {
        let reader = reader(Vec::new(), false);
        let recorder = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn OwnStorePaymentObserver>> = vec![recorder];

        let before = Utc::now();
        reconcile_once(&reader, &observers, DEFAULT_LOOKBACK, DEFAULT_LIMIT).await;

        let asked = reader.asked_since.lock().unwrap().unwrap();
        let looked_back = before - asked;
        assert!(
            looked_back >= chrono::Duration::hours(12),
            "looked back only {looked_back}, which cannot cover an invoice's life"
        );
        assert!(
            looked_back > chrono::Duration::from_std(DEFAULT_INTERVAL).unwrap() * 10,
            "the window must be far wider than the interval, not merely wider"
        );
    }

    /// A failed read is survivable. The next pass is minutes away and does
    /// the same work, so a database blip must not take the loop down or spin
    /// it against a database that is already struggling.
    #[tokio::test]
    async fn a_failed_read_dispatches_nothing_and_does_not_panic() {
        let recorder = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn OwnStorePaymentObserver>> = vec![recorder.clone()];

        let n = reconcile_once(
            &reader(vec![payment("a")], true),
            &observers,
            DEFAULT_LOOKBACK,
            DEFAULT_LIMIT,
        )
        .await;

        assert_eq!(n, 0);
        assert!(recorder.0.lock().unwrap().is_empty());
    }

    /// With nothing to tell, the read is not even attempted - an instance
    /// with no billing plugin should not query its own invoices every five
    /// minutes forever.
    #[tokio::test]
    async fn no_observers_means_no_read() {
        let reader = reader(vec![payment("a")], false);
        let n = reconcile_once(&reader, &[], DEFAULT_LOOKBACK, DEFAULT_LIMIT).await;

        assert_eq!(n, 0);
        assert!(
            reader.asked_since.lock().unwrap().is_none(),
            "the database must not be queried when there is nobody to tell"
        );
    }
}
