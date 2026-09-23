#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `reconcile_cursors` decides, per chain, which stored cursors are still
//! safe to resume from. `HashMap` iteration order is unspecified, so the
//! two-chains-disagree case an earlier version got wrong by sampling one
//! arbitrary entry can't be forced reliably through `EventConsumer::run` -
//! these call the method directly with a hand-built map instead.

use std::collections::HashMap;
use std::sync::Arc;

use data_service::{ChainCursor, InMemoryDataService};
use evm::monitor::bridge::{EventCursor, MemoryBridge};

use super::helpers::create_test_consumer;

#[tokio::test]
async fn a_stale_chains_cursor_does_not_lower_the_resume_point_below_a_current_chains() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge);

    let old_epoch = 1;
    let new_epoch = 2;

    // Chain 1 already recommitted under the new lineage after a break, with
    // only a few events applied so far. Chain 2 is idle and still carries a
    // cursor from before the break - one whose `seq`, taken at face value in
    // the new lineage's own numbering, is *lower* than chain 1's. Blending
    // it into a shared low-water mark (as picking one arbitrary chain's
    // epoch as representative for the whole map used to do) would move the
    // resume point backward to a position the new lineage hasn't actually
    // reached applying for chain 1 yet, silently skipping real events below
    // it once the stream catches up.
    let mut cursors = HashMap::from([
        (
            1,
            ChainCursor {
                epoch: new_epoch,
                seq: 40,
                block_height: 0,
            },
        ),
        (
            2,
            ChainCursor {
                epoch: old_epoch,
                seq: 3,
                block_height: 0,
            },
        ),
    ]);

    let resume = consumer
        .reconcile_cursors(&mut cursors, new_epoch)
        .await
        .unwrap();

    assert_eq!(
        resume,
        Some(EventCursor {
            epoch: new_epoch,
            seq: 40,
            block_height: 0,
        }),
        "chain 2's stale, pre-break seq must not lower the resume point below what chain 1 \
         has actually applied on the new lineage"
    );
    assert!(
        !cursors.contains_key(&2),
        "chain 2's stale cursor must be forgotten, not left around to be blended in again"
    );
    assert_eq!(
        cursors.get(&1).map(|c| c.seq),
        Some(40),
        "chain 1's own cursor is trustworthy and must survive reconciliation untouched"
    );
    assert_eq!(
        ds.watch_reset_calls(),
        1,
        "only the mismatched chain should have watch_retry re-armed"
    );
}

#[tokio::test]
async fn the_resume_point_is_the_low_water_mark_across_every_chain_still_on_the_current_epoch() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge);

    let epoch = 7;
    let mut cursors = HashMap::from([
        (
            1,
            ChainCursor {
                epoch,
                seq: 40,
                block_height: 0,
            },
        ),
        (
            2,
            ChainCursor {
                epoch,
                seq: 12,
                block_height: 0,
            },
        ),
    ]);

    let resume = consumer
        .reconcile_cursors(&mut cursors, epoch)
        .await
        .unwrap();

    assert_eq!(
        resume,
        Some(EventCursor {
            epoch,
            seq: 12,
            block_height: 0,
        }),
        "resuming from anything past chain 2's own committed position would skip events it \
         has not applied yet"
    );
    assert_eq!(cursors.len(), 2, "no chain's cursor was stale here");
    assert_eq!(
        ds.watch_reset_calls(),
        0,
        "nothing mismatched, so nothing should have been re-armed"
    );
}

#[tokio::test]
async fn every_chain_mismatching_resumes_from_scratch_rather_than_a_stale_combined_mark() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge);

    let old_epoch = 1;
    let new_epoch = 2;
    let mut cursors = HashMap::from([
        (
            1,
            ChainCursor {
                epoch: old_epoch,
                seq: 40,
                block_height: 0,
            },
        ),
        (
            2,
            ChainCursor {
                epoch: old_epoch,
                seq: 12,
                block_height: 0,
            },
        ),
    ]);

    let resume = consumer
        .reconcile_cursors(&mut cursors, new_epoch)
        .await
        .unwrap();

    assert_eq!(
        resume, None,
        "no chain's stored cursor names a lineage the outbox still has, so there is nothing \
         safe to resume from"
    );
    assert!(cursors.is_empty());
    assert_eq!(ds.watch_reset_calls(), 2);
}
