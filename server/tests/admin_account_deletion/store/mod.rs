//! `DELETE /admin/stores/{id}`, against a real database.
//!
//! Split into `basic` (name/ownership/payout/refund gating) and `monitor`
//! (the two tests that need a live `RedisEVMMonitor` to prove the unwatch
//! step actually fires, rather than merely that the delete succeeds).

mod basic;
mod monitor;
