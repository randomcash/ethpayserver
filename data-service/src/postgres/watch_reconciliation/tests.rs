//! Unit tests for the comparison itself.
//!
//! These cover the shape of the diff. They do **not** prove the feature works
//! against a real monitor: the database side and the live read are exercised
//! together in the integration test that seeds both stores.

use super::{WatchKey, compare_watches};
use types::ChainId;

const ADDR: &str = "0x00000000000000000000000000000000000000aa";
const TOKEN: &str = "0x00000000000000000000000000000000000000bb";

fn native(invoice: &str) -> WatchKey {
    WatchKey::new(ADDR, invoice, ChainId::evm(1), None)
}

fn token(invoice: &str) -> WatchKey {
    WatchKey::new(ADDR, invoice, ChainId::evm(1), Some(TOKEN))
}

#[test]
fn agreement_reports_nothing_in_either_direction() {
    let both = vec![native("inv-1"), token("inv-1")];
    let report = compare_watches(both.clone(), both);

    assert!(report.agrees());
    assert_eq!(report.stale_count(), 0);
    assert_eq!(report.missing_count(), 0);
}

#[test]
fn the_two_directions_are_never_merged() {
    let expected = vec![native("inv-1")];
    let live = vec![native("inv-2")];

    let report = compare_watches(expected, live);

    // One watch each way, for two different invoices - not "two
    // discrepancies", and not one cancelling the other out.
    assert_eq!(report.missing, vec![native("inv-1")]);
    assert_eq!(report.stale, vec![native("inv-2")]);
    assert!(!report.agrees());
}

/// The case the whole feature turns on. An address keeps its native watch and
/// loses its token watch; a comparison on the address alone would call that
/// agreement, and a payment in that token would arrive uncredited.
#[test]
fn a_lost_token_watch_is_missing_even_though_the_address_is_still_watched() {
    let expected = vec![native("inv-1"), token("inv-1")];
    let live = vec![native("inv-1")];

    let report = compare_watches(expected, live);

    assert_eq!(report.missing, vec![token("inv-1")]);
    assert_eq!(report.stale_count(), 0);
}

/// And the reverse: a token watch nobody expects is not excused by the native
/// watch on the same address being legitimate.
#[test]
fn an_unexpected_token_watch_is_stale_even_though_the_address_is_expected() {
    let expected = vec![native("inv-1")];
    let live = vec![native("inv-1"), token("inv-1")];

    let report = compare_watches(expected, live);

    assert_eq!(report.stale, vec![token("inv-1")]);
    assert_eq!(report.missing_count(), 0);
}

/// The same watch on two chains is two watches, not one.
#[test]
fn the_chain_is_part_of_the_key() {
    let expected = vec![WatchKey::new(ADDR, "inv-1", ChainId::evm(1), None)];
    let live = vec![WatchKey::new(ADDR, "inv-1", ChainId::evm(137), None)];

    let report = compare_watches(expected, live);

    assert_eq!(report.missing_count(), 1);
    assert_eq!(report.stale_count(), 1);
}

/// A checksummed address in the database and the monitor's lower-cased key are
/// the same watch. Without normalisation every single watch would be reported
/// twice - once stale, once missing - and the report would be unreadable, which
/// is the state in which a real discrepancy goes unnoticed.
#[test]
fn casing_does_not_invent_a_discrepancy() {
    let expected = vec![WatchKey::new(
        &ADDR.to_uppercase().replace("0X", "0x"),
        "inv-1",
        ChainId::evm(1),
        Some(&TOKEN.to_uppercase().replace("0X", "0x")),
    )];
    let live = vec![token("inv-1")];

    assert!(compare_watches(expected, live).agrees());
}
