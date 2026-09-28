//! The orchestrator's durable resume point into an event adapter's outbox.
//!
//! Redis pub/sub (still used for the command direction) drops a message
//! with no subscriber connected; nothing recovers it. The event direction
//! instead publishes into a durable, replayable outbox (see
//! `evm::monitor::bridge::EventBridge::subscribe_from`), and this is the
//! other half: the position this server has actually applied, so a restart
//! resumes instead of silently picking up wherever the outbox happens to be
//! "now".
//!
//! Lives here rather than in the shared `types` repository traits for the
//! same reason `reorg.rs` does: this is this server's bookkeeping over this
//! server's schema, not a concept the wider product shares.

use async_trait::async_trait;
use types::RepositoryResult;

/// One adapter's last-applied position for one chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainCursor {
    pub epoch: i64,
    pub seq: i64,
    pub block_height: i64,
}

/// Reads durable resume cursors.
#[async_trait]
pub trait ChainCursorReader: Send + Sync {
    /// Every chain this server has a cursor for, under `adapter_id`.
    ///
    /// Empty for a chain that has never had an event applied - not a `0`
    /// cursor, because `0` is a real, resumable position and "never seen
    /// this chain" must not be confused with it.
    async fn chain_cursors(
        &self,
        adapter_id: &str,
    ) -> RepositoryResult<std::collections::HashMap<u64, ChainCursor>>;
}

/// Commits durable resume cursors.
#[async_trait]
pub trait ChainCursorWriter: Send + Sync {
    /// Record `cursor` as applied for `(adapter_id, chain_id)`.
    ///
    /// Called only after the event at `cursor` has already been applied to
    /// invoice/payment state - ordering it any earlier would let a crash
    /// between the write and the apply lose the event while still telling
    /// the next resume it was handled.
    async fn commit_chain_cursor(
        &self,
        adapter_id: &str,
        chain_id: u64,
        cursor: ChainCursor,
    ) -> RepositoryResult<()>;

    /// Re-arm `watch_retry` for every active watch on `chain_id`.
    ///
    /// Called when the outbox's epoch no longer matches what this server
    /// last saw for the chain, meaning whatever it missed in the gap cannot
    /// be replayed - there is no cursor left that names the right lineage.
    /// This does not recover a payment that confirmed and expired entirely
    /// inside that gap, but it does get every still-open watch re-sent to
    /// the adapter within `watch_retry`'s normal 30-second cycle, the same
    /// path a fresh watch already takes and already has tests.
    async fn reset_chain_watch_notifications(&self, chain_id: u64) -> RepositoryResult<u64>;
}
