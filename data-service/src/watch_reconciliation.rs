//! Comparing what should be watched (Postgres) against what is actually
//! watched (the monitor's Redis key set).
//!
//! Deliberately independent of both storage backends: neither feature flag
//! gates this module, so the comparison itself is testable without a
//! database or a Redis connection. Only the two sides that feed it -
//! `postgres::expected_watch` and the `redis` feature's
//! `LiveWatchedAddressReader::get_all_watched` - need real infrastructure.

use std::collections::HashSet;

/// Identity of a watched address, independent of which side observed it.
///
/// Deliberately excludes the invoice id: the same `(chain, address, token)`
/// triple is the unique key on both sides (Redis keys on exactly this, and
/// `watched_addresses` carries `CONSTRAINT unique_watched_address UNIQUE
/// (address, chain_id, token_address)` with no `is_active` qualifier - so a
/// second invoice can never claim the same triple in Postgres while an
/// earlier row for it still exists, live or not; the earlier row has to be
/// gone first).
///
/// That ordering is what makes dropping the invoice id safe rather than
/// merely convenient: the moment an address is legitimately rewatched for a
/// new invoice, the application issues the same `watch_address` call that
/// created the original watch, and `RedisDataService::watch_address` does an
/// unconditional `SET` on that exact key - not `SETNX`, not an append to a
/// list - so the new invoice id replaces whatever was there, including a
/// stale one this reconciler had not yet caught. By the time both sides can
/// agree the key is "expected" again, Redis is already holding the new
/// invoice, not the old one; there is no window where the key matches but
/// the value behind it is wrong. `server/tests/watch_reconciliation.rs`
/// proves this end to end: a stale watch is seeded, reported, then the same
/// address is legitimately rewatched for a second invoice, and the
/// reconciler reports nothing while the Redis value has in fact moved to the
/// new invoice.
///
/// Address and token are lower-cased so a checksum mismatch between the two
/// sources never manufactures a false stale/missed pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WatchKey {
    chain_id: types::ChainId,
    address: String,
    token_address: Option<String>,
}

impl WatchKey {
    pub fn new(chain_id: types::ChainId, address: &str, token_address: Option<&str>) -> Self {
        Self {
            chain_id,
            address: address.to_lowercase(),
            token_address: token_address.map(str::to_lowercase),
        }
    }
}

/// The result of comparing an expected watch set against an actual one.
///
/// The two directions are different faults, not two halves of one count: a
/// stale watch wastes RPC calls on something already resolved, while a
/// missed watch means a real payment can arrive with nobody watching for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchReconciliation {
    /// Watched in Redis, absent from the expected set - a deleted or
    /// resolved invoice the monitor is still polling.
    pub stale: Vec<WatchKey>,
    /// In the expected set, not watched in Redis - a live invoice nobody is
    /// watching.
    pub missed: Vec<WatchKey>,
}

/// Diff an expected watch set against what is actually being watched.
pub fn reconcile(expected: &[WatchKey], actual: &[WatchKey]) -> WatchReconciliation {
    let expected_set: HashSet<&WatchKey> = expected.iter().collect();
    let actual_set: HashSet<&WatchKey> = actual.iter().collect();

    WatchReconciliation {
        stale: actual_set
            .difference(&expected_set)
            .map(|k| (*k).clone())
            .collect(),
        missed: expected_set
            .difference(&actual_set)
            .map(|k| (*k).clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(chain: u64, address: &str) -> WatchKey {
        WatchKey::new(types::ChainId::evm(chain), address, None)
    }

    /// The vacuous case this ticket exists to avoid: two identical sets must
    /// never report either direction, or every reconciliation would be a
    /// false positive.
    #[test]
    fn identical_sets_report_nothing() {
        let set = vec![key(1, "0xabc"), key(137, "0xdef")];
        let diff = reconcile(&set, &set);
        assert!(diff.stale.is_empty());
        assert!(diff.missed.is_empty());
    }

    /// Proof this can go red: a watch present in Redis but absent from the
    /// expected set - a deleted or resolved invoice still being polled.
    #[test]
    fn a_watch_actual_has_and_expected_does_not_is_reported_stale() {
        let expected = vec![key(1, "0xabc")];
        let actual = vec![key(1, "0xabc"), key(1, "0xstale")];

        let diff = reconcile(&expected, &actual);
        assert_eq!(diff.stale, vec![key(1, "0xstale")]);
        assert!(diff.missed.is_empty());
    }

    /// Proof this can go red the other way: a watch the expected set has but
    /// Redis does not - a live invoice nobody is watching.
    #[test]
    fn a_watch_expected_has_and_actual_does_not_is_reported_missed() {
        let expected = vec![key(1, "0xabc"), key(1, "0xmissed")];
        let actual = vec![key(1, "0xabc")];

        let diff = reconcile(&expected, &actual);
        assert!(diff.stale.is_empty());
        assert_eq!(diff.missed, vec![key(1, "0xmissed")]);
    }

    /// A mismatch on chain id alone must count, even with an identical
    /// address - two chains do not share a watch just because they share a
    /// string.
    #[test]
    fn the_same_address_on_a_different_chain_is_not_a_match() {
        let expected = vec![key(1, "0xabc")];
        let actual = vec![key(137, "0xabc")];

        let diff = reconcile(&expected, &actual);
        assert_eq!(diff.stale.len(), 1);
        assert_eq!(diff.missed.len(), 1);
    }

    /// Redis lower-cases what it stores and Postgres keeps the checksummed
    /// form - a case difference alone must not manufacture a false positive
    /// in either direction.
    #[test]
    fn address_comparison_is_case_insensitive() {
        let expected = vec![WatchKey::new(
            types::ChainId::evm(1),
            "0xABCDEF1234567890ABCDEF1234567890ABCDEF12",
            None,
        )];
        let actual = vec![WatchKey::new(
            types::ChainId::evm(1),
            "0xabcdef1234567890abcdef1234567890abcdef12",
            None,
        )];

        let diff = reconcile(&expected, &actual);
        assert!(diff.stale.is_empty());
        assert!(diff.missed.is_empty());
    }

    /// A token address distinguishes two watches on the same address - an
    /// ERC20 watch must not be mistaken for the native-asset one.
    #[test]
    fn a_token_address_distinguishes_otherwise_identical_watches() {
        let native = WatchKey::new(types::ChainId::evm(1), "0xabc", None);
        let erc20 = WatchKey::new(types::ChainId::evm(1), "0xabc", Some("0xusdc"));

        let diff = reconcile(&[native], &[erc20]);
        assert_eq!(diff.stale.len(), 1);
        assert_eq!(diff.missed.len(), 1);
    }
}
