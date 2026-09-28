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
/// Includes the invoice id, deliberately. The `(chain, address, token)`
/// triple alone is the unique key on both sides at rest - Redis keys on
/// exactly this, and `watched_addresses` carries `CONSTRAINT
/// unique_watched_address UNIQUE (address, chain_id, token_address)` - but
/// "at rest" is not the state this reconciler needs to be right about. A
/// legitimate reuse writes the two stores in two separate, non-atomic steps:
/// `create_invoice_with_options` commits the Postgres side first, and only
/// afterward does the application call `watch_address` to overwrite the
/// Redis key. Between those two calls, Postgres already reports the address
/// expected for the new invoice while Redis still holds the old one - same
/// triple, different invoice behind it. A triple-only key cannot see that:
/// it would report no discrepancy while a payment landing in that window
/// would be credited to the wrong invoice. Keying on the invoice id as well
/// turns that window into exactly the two-entry diff it should be - one
/// `missed` (the new invoice, not yet in Redis) and one `stale` (the old
/// invoice, still there) - rather than a silent match neither side flags.
///
/// `server/tests/watch_reconciliation.rs` proves both ends of this: a stale
/// watch is seeded, reported, then legitimately reused for a second invoice.
/// Caught mid-reuse (Postgres updated, Redis not yet told), it reports the
/// missed/stale pair above; once the Redis write lands, both sides agree on
/// the new invoice and the reconciler goes quiet for that key.
///
/// Address and token are lower-cased so a checksum mismatch between the two
/// sources never manufactures a false stale/missed pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WatchKey {
    chain_id: types::ChainId,
    address: String,
    token_address: Option<String>,
    invoice_id: types::InvoiceId,
}

impl WatchKey {
    pub fn new(
        chain_id: types::ChainId,
        address: &str,
        token_address: Option<&str>,
        invoice_id: types::InvoiceId,
    ) -> Self {
        Self {
            chain_id,
            address: address.to_lowercase(),
            token_address: token_address.map(str::to_lowercase),
            invoice_id,
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
        keyed(chain, address, "inv-default")
    }

    fn keyed(chain: u64, address: &str, invoice_id: &str) -> WatchKey {
        WatchKey::new(
            types::ChainId::evm(chain),
            address,
            None,
            types::InvoiceId::from_string(invoice_id.to_string()),
        )
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
            types::InvoiceId::from_string("inv-1".to_string()),
        )];
        let actual = vec![WatchKey::new(
            types::ChainId::evm(1),
            "0xabcdef1234567890abcdef1234567890abcdef12",
            None,
            types::InvoiceId::from_string("inv-1".to_string()),
        )];

        let diff = reconcile(&expected, &actual);
        assert!(diff.stale.is_empty());
        assert!(diff.missed.is_empty());
    }

    /// A token address distinguishes two watches on the same address - an
    /// ERC20 watch must not be mistaken for the native-asset one.
    #[test]
    fn a_token_address_distinguishes_otherwise_identical_watches() {
        let native = WatchKey::new(
            types::ChainId::evm(1),
            "0xabc",
            None,
            types::InvoiceId::from_string("inv-1".to_string()),
        );
        let erc20 = WatchKey::new(
            types::ChainId::evm(1),
            "0xabc",
            Some("0xusdc"),
            types::InvoiceId::from_string("inv-1".to_string()),
        );

        let diff = reconcile(&[native], &[erc20]);
        assert_eq!(diff.stale.len(), 1);
        assert_eq!(diff.missed.len(), 1);
    }

    /// The case this key design exists for: the same `(chain, address,
    /// token)` triple, caught mid-reuse - Postgres already reassigned to a
    /// new invoice, Redis still holding the old one. A triple-only key would
    /// see this as a match and report nothing, silently misattributing any
    /// payment that lands in that window. Keying on the invoice id turns it
    /// into a visible pair instead: the new invoice is missed, the old one
    /// is stale.
    #[test]
    fn a_reused_address_mid_reuse_is_one_missed_and_one_stale_not_a_silent_match() {
        let expected = vec![keyed(1, "0xabc", "invoice-b")];
        let actual = vec![keyed(1, "0xabc", "invoice-a")];

        let diff = reconcile(&expected, &actual);
        assert_eq!(diff.missed, vec![keyed(1, "0xabc", "invoice-b")]);
        assert_eq!(diff.stale, vec![keyed(1, "0xabc", "invoice-a")]);
    }
}
