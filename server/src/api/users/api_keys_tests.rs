use super::*;

/// A caller written before the permissions field existed sends no list at all,
/// and must still be able to create a key.
#[test]
fn an_omitted_scope_is_not_a_narrowing_request() {
    assert!(!asks_to_narrow_the_key(&[]));
}

/// `["unrestricted"]` asks for the owner's role in full, which is what every
/// key this server issues actually carries.
#[test]
fn an_explicit_unrestricted_scope_is_honoured() {
    assert!(!asks_to_narrow_the_key(&[
        Policies::UNRESTRICTED.to_string()
    ]));
}

/// A real policy string asks for less than the owner's role. This server
/// cannot issue such a key, so the request is refused rather than served with
/// a wider one than was asked for.
#[test]
fn a_real_policy_is_a_narrowing_request() {
    assert!(asks_to_narrow_the_key(&[
        Policies::STORE_VIEW_INVOICES.to_string()
    ]));
}

/// `unrestricted` alongside anything else is still narrowing: the caller is
/// describing a set of policies, and the only set this server can produce is
/// the owner's whole role.
#[test]
fn unrestricted_mixed_with_a_policy_is_still_narrowing() {
    assert!(asks_to_narrow_the_key(&[
        Policies::UNRESTRICTED.to_string(),
        Policies::STORE_VIEW_INVOICES.to_string(),
    ]));
}
