//! What the post-delete unwatch counter counts, and what it cannot see.
//!
//! [`super::record_unwatch_failed`] is the only signal that a deletion left a
//! watch behind, and it fires on two branches of
//! [`super::unwatch_after_delete`]: the per-address branch where the unwatch
//! command cannot be published, and the branch where no monitor is wired into
//! this process at all - which leaves exactly the stale watch the counter
//! exists for and otherwise only writes a warning. Neither had a test, so
//! neither had been shown capable of moving.
//!
//! These are unit tests in the crate rather than `#[ignore]`d integration
//! tests, for two reasons. A publish failure needs a monitor that fails, and
//! the one real implementation cannot be put in that state from a test -
//! building it at all requires a reachable live-watch store. And an
//! `#[ignore]`d test runs only in the pass that needs a database, where the
//! suite's own convention is to return early when the variable is unset; these
//! run in the ordinary workspace test step and so cannot be skipped into a
//! green. That the no-monitor branch is *reached* by a real endpoint is a
//! separate claim, covered by an integration test in `server/tests/`.
//!
//! The counter is read through a recorder local to the calling thread, never
//! the process-wide one: a global recorder can be installed only once per
//! process, and this crate's own metrics tests already contend for it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use evm::Address;
use evm::monitor::ChainHealth;
use metrics_exporter_prometheus::PrometheusBuilder;
use types::ChainId;
use uuid::Uuid;

use crate::services::evm_monitor::{EVMMonitor, EVMMonitorError};

use super::tests::cleanup_info;
use super::unwatch_after_delete;

const COUNTER: &str = "ethpayserver_unwatch_after_delete_failures_total";

/// A monitor that records every unwatch asked of it, and either publishes the
/// command or fails to.
///
/// `publishes: false` is not a contrived state. The command travels over
/// pub/sub to a different process, so a store that is unreachable or refusing
/// writes is the ordinary way this fails in production; it just cannot be
/// reproduced with the real implementation, which needs a reachable store
/// before it can be constructed.
struct TestMonitor {
    publishes: bool,
    unwatch_calls: AtomicUsize,
}

impl TestMonitor {
    fn publishing() -> Self {
        Self {
            publishes: true,
            unwatch_calls: AtomicUsize::new(0),
        }
    }

    fn failing() -> Self {
        Self {
            publishes: false,
            unwatch_calls: AtomicUsize::new(0),
        }
    }

    fn unwatch_calls(&self) -> usize {
        self.unwatch_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EVMMonitor for TestMonitor {
    async fn watch_address(
        &self,
        _chain_id: &ChainId,
        _address: Address,
        _invoice_id: Uuid,
        _expected_amount: Option<evm::U256>,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        unimplemented!("the delete path only ever unwatches")
    }

    async fn watch_address_by_chain_id(
        &self,
        _chain_id: u64,
        _address: Address,
        _invoice_id: Uuid,
        _expected_amount: Option<evm::U256>,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        unimplemented!("the delete path only ever unwatches")
    }

    async fn unwatch_address(
        &self,
        _chain_id: &ChainId,
        _address: Address,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        unimplemented!("the delete path calls the chain-id form")
    }

    async fn unwatch_address_by_chain_id(
        &self,
        _chain_id: u64,
        _address: Address,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        self.unwatch_calls.fetch_add(1, Ordering::SeqCst);
        if self.publishes {
            Ok(())
        } else {
            Err(EVMMonitorError::Bridge(evm::EvmError::Monitor(
                "could not publish the unwatch command".to_string(),
            )))
        }
    }

    async fn health_check(&self) -> Result<(), EVMMonitorError> {
        unimplemented!("not read by the delete path")
    }

    async fn get_chain_health(&self) -> Result<Vec<ChainHealth>, EVMMonitorError> {
        unimplemented!("not read by the delete path")
    }
}

/// Run `f` against a recorder nothing else shares, and return how many times
/// the counter fired while it ran.
///
/// `block_on` on this thread, not a spawned runtime: a local recorder is
/// thread-local, so a future polled on another thread would record into the
/// process-wide recorder instead and this would read zero no matter what the
/// code did.
fn unwatch_failures_during<F: Future<Output = ()>>(f: F) -> u64 {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::with_local_recorder(&recorder, || futures::executor::block_on(f));
    counter_value(&handle.render())
}

/// Absent from the render means the counter never fired, which is genuinely
/// zero. A value that is present but unreadable is not: reporting that as zero
/// would be the could-not-look-versus-found-nothing conflation this repository
/// has paid for repeatedly, so it panics instead.
fn counter_value(rendered: &str) -> u64 {
    let mut found: Option<u64> = None;
    for line in rendered.lines() {
        // `# HELP`/`# TYPE` lines start with `#` and so never match.
        let Some(rest) = line.strip_prefix(COUNTER) else {
            continue;
        };
        // A space separates name from value; anything else is a longer metric
        // name that merely starts with this one.
        let Some(value) = rest.strip_prefix(' ') else {
            continue;
        };
        found = Some(
            value
                .trim()
                .parse()
                .unwrap_or_else(|e| panic!("could not read {COUNTER} from {line:?}: {e}")),
        );
    }
    found.unwrap_or(0)
}

fn evm_row(address: &str) -> data_service::CleanupAddressInfo {
    cleanup_info(address, None, ChainId::evm(11155111))
}

/// The per-address error path. Two watched addresses, neither command
/// publishable, so the counter must stand at two - and the second must have
/// been attempted at all, which is what separates "counts every failure" from
/// "stops at the first one and counts it".
#[test]
fn a_publish_failure_is_counted_once_per_address_and_stops_nothing() {
    let monitor = TestMonitor::failing();
    let rows = vec![
        evm_row("0x1111111111111111111111111111111111111111"),
        evm_row("0x2222222222222222222222222222222222222222"),
    ];

    let failures = unwatch_failures_during(unwatch_after_delete(Some(&monitor), rows));

    assert_eq!(
        failures, 2,
        "each address whose unwatch could not be published must be counted"
    );
    assert_eq!(
        monitor.unwatch_calls(),
        2,
        "a failure on the first address must not abandon the rest of the batch"
    );
}

/// The branch that would otherwise be silent: no monitor in this process, so
/// nothing is even attempted and the stale watch is left exactly as it is.
/// Before this counter that state produced a warning and no signal at all.
#[test]
fn no_monitor_wired_counts_every_watch_it_left_behind() {
    let rows = vec![
        evm_row("0x3333333333333333333333333333333333333333"),
        evm_row("0x4444444444444444444444444444444444444444"),
        evm_row("0x5555555555555555555555555555555555555555"),
    ];

    let failures = unwatch_failures_during(unwatch_after_delete(None::<&TestMonitor>, rows));

    assert_eq!(
        failures, 3,
        "with no monitor wired, every address is left watched and every one counts"
    );
}

/// The ordinary case a deletion is in: nothing was watched, so nothing was
/// left behind and there is nothing to count. Without this the two tests above
/// would pass equally well against a counter that fired unconditionally.
#[test]
fn no_monitor_and_no_watched_addresses_counts_nothing() {
    let failures = unwatch_failures_during(unwatch_after_delete(None::<&TestMonitor>, Vec::new()));

    assert_eq!(failures, 0);
}

/// What a zero actually rules out, stated as a test so no reader has to take
/// the doc comment's word for it: the counter stays at zero once the command
/// is *published*, and publishing is all that was established here. Nothing
/// acknowledged the command, nothing acted on it, and no subscriber had to
/// exist. A monitor that received it and dropped it is indistinguishable from
/// this, and reads zero too.
#[test]
fn a_published_unwatch_counts_nothing_even_though_nothing_acknowledged_it() {
    let monitor = TestMonitor::publishing();
    let rows = vec![evm_row("0x6666666666666666666666666666666666666666")];

    let failures = unwatch_failures_during(unwatch_after_delete(Some(&monitor), rows));

    assert_eq!(monitor.unwatch_calls(), 1, "the command must have gone out");
    assert_eq!(
        failures, 0,
        "a published command counts nothing, whether or not anything received it"
    );
}

/// The third way a stale watch survives, pinned rather than left to be
/// inferred: a row `parse_watch_target` rejects is skipped without a command
/// being built, and is deliberately not counted. The realistic case is a watch
/// on a non-EVM chain, which this monitor never held, so counting it would
/// report a failure that did not happen. A malformed address would be worth
/// counting and is indistinguishable from that case here, which is the cost of
/// the choice.
#[test]
fn a_row_the_monitor_cannot_address_is_skipped_without_counting() {
    let monitor = TestMonitor::failing();
    let rows = vec![
        cleanup_info(
            "0x7777777777777777777777777777777777777777",
            None,
            ChainId::new("tron", "728126428").expect("valid chain id"),
        ),
        cleanup_info("not-an-address", None, ChainId::evm(11155111)),
    ];

    let failures = unwatch_failures_during(unwatch_after_delete(Some(&monitor), rows));

    assert_eq!(
        monitor.unwatch_calls(),
        0,
        "neither row can be turned into a command, so none should be attempted"
    );
    assert_eq!(
        failures, 0,
        "a skipped row is not counted - see record_unwatch_failed's docs for why"
    );
}
