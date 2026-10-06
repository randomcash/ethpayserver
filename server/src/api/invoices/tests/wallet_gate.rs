#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::*;
use ::types::ChainId;

/// A payment method that resolves to a wallet, or one whose pin -> store
/// override -> account primary walk ran out - `wallet_id: None` is exactly
/// what `get_enabled_payment_methods` reports in that case (see
/// `StorePaymentMethod::wallet_id`).
fn make_pm(wallet_id: Option<uuid::Uuid>) -> data_service::StorePaymentMethod {
    data_service::StorePaymentMethod {
        id: uuid::Uuid::new_v4(),
        store_id: uuid::Uuid::new_v4(),
        chain_id: ChainId::evm(1),
        token_address: None,
        asset_symbol: "ETH".to_string(),
        decimals: 18,
        wallet_id,
        xpub: wallet_id.map(|_| "xpub_test".to_string()),
        derivation_index: wallet_id.map(|_| 0),
        enabled: true,
        created_at: chrono::Utc::now(),
    }
}

/// A gate that refused every store would pass this trivially, so this is the
/// test that would fail without it: nothing here resolves, and the ticket's
/// whole point is that such a store cannot create an invoice.
#[test]
fn a_store_where_no_method_resolves_has_no_wallet() {
    let methods = vec![make_pm(None), make_pm(None)];
    assert!(store_has_no_wallet(&methods));
}

/// The gate must not catch a store that can actually be paid. Mixing one
/// resolved method in with unresolved ones also guards against a gate that
/// checks only the first entry.
#[test]
fn a_store_with_any_resolvable_wallet_is_unaffected() {
    let methods = vec![make_pm(None), make_pm(Some(uuid::Uuid::new_v4()))];
    assert!(!store_has_no_wallet(&methods));
}
