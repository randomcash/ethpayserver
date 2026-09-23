#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `run` has three places where it used to give up quietly - a failed
//! `chain_cursors` load, a failed `current_epoch` read, and a second
//! `subscribe_from` failure after the one out-of-range retry - plus
//! `break_lineage` swallowing a failed `reset_chain_watch_notifications`.
//! Every one of those left the process running with no consumer at all,
//! indistinguishable from a healthy, idle one: nothing paged anyone, and
//! nothing in the diff proved a restart would even help. These assert that
//! all four now go through `ResumeFailureHook` instead of returning silently.
//! Production leaves the hook unset, which exits the process so a supervisor
//! restarts it, the same recovery `ApplyFailureHook` already had.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use data_service::{ChainCursor, ChainCursorWriter, InMemoryDataService};
use evm::monitor::bridge::{CommandStream, DurableEventStream, EventBridge, EventCursor};
use evm::monitor::events::{MonitorCommand, MonitorEvent};
use evm::{EvmError, EvmResult};

use super::super::ADAPTER_ID;
use super::helpers::{create_test_consumer, create_test_consumer_with_bridge};

/// Records every call to `hook` and lets a test wait for the first one
/// without polling `Mutex` state on a tight loop.
async fn wait_for_reason(reasons: &Mutex<Vec<String>>) -> String {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(reason) = reasons.lock().unwrap().first().cloned() {
                return reason;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("resume failure hook was never called")
}

#[tokio::test]
async fn a_failed_chain_cursors_load_halts_instead_of_running_with_a_dead_consumer() {
    let ds = Arc::new(InMemoryDataService::new());
    ds.set_fail_chain_cursors(true);
    let bridge = Arc::new(evm::monitor::bridge::MemoryBridge::new());

    let reasons: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = reasons.clone();
    let consumer =
        create_test_consumer(ds, bridge).with_resume_failure_hook(Arc::new(move |reason| {
            recorded.lock().unwrap().push(reason.to_string())
        }));

    let task = tokio::spawn(consumer.run());
    let reason = wait_for_reason(&reasons).await;
    assert!(
        reason.contains("chain cursors"),
        "unexpected reason: {reason}"
    );
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
}

/// A bridge whose `current_epoch` always fails. Every other method panics if
/// called: a failure this early must stop `run` before it ever touches
/// `subscribe_from`.
struct FailingEpochBridge;

#[async_trait]
impl EventBridge for FailingEpochBridge {
    async fn publish(&self, _event: &MonitorEvent) -> EvmResult<()> {
        unimplemented!("not exercised by this test")
    }

    async fn subscribe_from(&self, _from: Option<EventCursor>) -> EvmResult<DurableEventStream> {
        panic!("run must not call subscribe_from after current_epoch fails")
    }

    async fn current_epoch(&self) -> EvmResult<i64> {
        Err(EvmError::Monitor(
            "simulated epoch read failure".to_string(),
        ))
    }

    async fn bump_epoch(&self) -> EvmResult<i64> {
        unimplemented!("not exercised by this test")
    }

    async fn publish_command(&self, _command: &MonitorCommand) -> EvmResult<()> {
        unimplemented!("not exercised by this test")
    }

    async fn subscribe_commands(&self) -> EvmResult<CommandStream> {
        unimplemented!("not exercised by this test")
    }

    fn name(&self) -> &str {
        "FailingEpochBridge"
    }

    async fn health_check(&self) -> EvmResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_failed_epoch_read_halts_before_ever_subscribing() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(FailingEpochBridge);

    let reasons: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = reasons.clone();
    let consumer = create_test_consumer_with_bridge(ds, bridge).with_resume_failure_hook(Arc::new(
        move |reason| recorded.lock().unwrap().push(reason.to_string()),
    ));

    let task = tokio::spawn(consumer.run());
    let reason = wait_for_reason(&reasons).await;
    assert!(reason.contains("epoch"), "unexpected reason: {reason}");
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
}

/// A bridge whose `subscribe_from` always reports the resume position as
/// out of range, so `run`'s bounded one-retry loop always exhausts itself.
struct AlwaysOutOfRangeBridge;

#[async_trait]
impl EventBridge for AlwaysOutOfRangeBridge {
    async fn publish(&self, _event: &MonitorEvent) -> EvmResult<()> {
        unimplemented!("not exercised by this test")
    }

    async fn subscribe_from(&self, _from: Option<EventCursor>) -> EvmResult<DurableEventStream> {
        Err(EvmError::EventStreamOutOfRange(
            "simulated permanent retention loss".to_string(),
        ))
    }

    async fn current_epoch(&self) -> EvmResult<i64> {
        Ok(1)
    }

    async fn bump_epoch(&self) -> EvmResult<i64> {
        Ok(2)
    }

    async fn publish_command(&self, _command: &MonitorCommand) -> EvmResult<()> {
        unimplemented!("not exercised by this test")
    }

    async fn subscribe_commands(&self) -> EvmResult<CommandStream> {
        unimplemented!("not exercised by this test")
    }

    fn name(&self) -> &str {
        "AlwaysOutOfRangeBridge"
    }

    async fn health_check(&self) -> EvmResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_second_out_of_range_after_the_one_retry_halts_rather_than_looping_forever() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(AlwaysOutOfRangeBridge);

    let reasons: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = reasons.clone();
    let consumer = create_test_consumer_with_bridge(ds, bridge).with_resume_failure_hook(Arc::new(
        move |reason| recorded.lock().unwrap().push(reason.to_string()),
    ));

    let task = tokio::spawn(consumer.run());
    let reason = wait_for_reason(&reasons).await;
    assert!(reason.contains("subscribe"), "unexpected reason: {reason}");
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
}

#[tokio::test]
async fn a_lineage_break_that_cannot_re_arm_watch_retry_halts_rather_than_forgetting_the_gap() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(evm::monitor::bridge::MemoryBridge::new());

    // Chain 1 has a stored cursor at epoch 1; bumping the bridge to epoch 2
    // makes `reconcile_cursors` see a mismatch and call `break_lineage` -
    // which this test then makes fail to re-arm `watch_retry` for it.
    ChainCursorWriter::commit_chain_cursor(
        &*ds,
        ADAPTER_ID,
        1,
        ChainCursor {
            epoch: 1,
            seq: 5,
            block_height: 0,
        },
    )
    .await
    .unwrap();
    bridge.bump_epoch().await.unwrap();
    ds.set_fail_reset_chain_watch_notifications(true);

    let reasons: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = reasons.clone();
    let consumer =
        create_test_consumer(ds, bridge).with_resume_failure_hook(Arc::new(move |reason| {
            recorded.lock().unwrap().push(reason.to_string())
        }));

    let task = tokio::spawn(consumer.run());
    let reason = wait_for_reason(&reasons).await;
    assert!(reason.contains("re-arm"), "unexpected reason: {reason}");
    let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
}
