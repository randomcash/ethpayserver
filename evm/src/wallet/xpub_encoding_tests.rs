use super::tests::{EVM_ACCOUNT_XPUB, TEST_MNEMONIC, TRON_ACCOUNT_XPUB};
use super::*;

/// `TEST_MNEMONIC` with the BIP-39 passphrase `"secret"` ("25th word"),
/// computed once from this crate's own derivation and pinned here as a
/// regression guard - not an externally-sourced vector like
/// `EVM_ACCOUNT_XPUB`/`TRON_ACCOUNT_XPUB` above, but different from both
/// of those by construction, since the seed a passphrase produces is
/// unrelated to the empty-passphrase seed. If `from_mnemonic` ever
/// dropped or mishandled the passphrase argument, this would collapse to
/// `EVM_ACCOUNT_XPUB`/`TRON_ACCOUNT_XPUB` instead and the assertion below
/// would fail.
const EVM_ACCOUNT_XPUB_WITH_PASSPHRASE: &str = "xpub6Bmqz11Kt5qtj3xbXZkzEyYw43EDFGCon5GzC4udf7DPugyKjVppdX2amQZrGs4rqAJH79pDtge2UDENZzjz9DgcV3WmfbwYAXj2epC5cgz";
const TRON_ACCOUNT_XPUB_WITH_PASSPHRASE: &str = "xpub6C7191Y1fJQN5X3REaqgCesHj2fhXCsLYuK6vf53MHyroMKGER3vX9NvfJmSz7MgCUnbMcUWYFiPAiuQpDD1AXQz5CMX9EuDtJb5of7vMU6";

/// `HdWallet` must export the exact string a real wallet would - not just
/// something `XpubDeriver` can parse back.
///
/// Every other test that touches `EVM_ACCOUNT_XPUB`/`TRON_ACCOUNT_XPUB`
/// hands the literal to `XpubDeriver::from_xpub`, which round-trips fine
/// no matter which BIP-32/49/84 encoding produced the bytes - the base58
/// version prefix only picks which of `xpub`/`ypub`/`zpub` comes out, and
/// parsing recovers it either way. That let `HdWallet::from_mnemonic`
/// default to the underlying library's `Hint::SegWit` and export `zpub…`
/// silently: every derived address stayed correct, `validate_xpub` still
/// accepted the result, and nothing here caught that the string handed to
/// a merchant no longer looked anything like the `xpub…` this repo's own
/// docs and API examples promise.
#[test]
fn account_xpub_string_matches_what_a_real_wallet_exports() {
    let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "").unwrap();
    assert_eq!(
        wallet.account_xpub_string_for(ChainFamily::Evm).unwrap(),
        EVM_ACCOUNT_XPUB
    );
    assert_eq!(
        wallet.account_xpub_string_for(ChainFamily::Tron).unwrap(),
        TRON_ACCOUNT_XPUB
    );
}

/// A non-empty BIP-39 passphrase ("25th word") is the `derive-xpub
/// from-existing` path a merchant with an existing seed phrase walks -
/// and it produces a different account, so a silent regression that
/// dropped or ignored the argument would export `EVM_ACCOUNT_XPUB`
/// instead of a merchant's real passphrase-protected key, with no error
/// anywhere in the chain (`validate_xpub` accepts either string). Pinned
/// at the xpub-string level, not just `derive_address` as
/// `test_passphrase_changes_addresses` already does, so a regression
/// that reached only the account-key export path and not per-index
/// derivation would still be caught.
#[test]
fn passphrase_changes_the_exported_account_xpub() {
    let wallet = HdWallet::from_mnemonic(TEST_MNEMONIC, "secret").unwrap();
    assert_eq!(
        wallet.account_xpub_string_for(ChainFamily::Evm).unwrap(),
        EVM_ACCOUNT_XPUB_WITH_PASSPHRASE
    );
    assert_eq!(
        wallet.account_xpub_string_for(ChainFamily::Tron).unwrap(),
        TRON_ACCOUNT_XPUB_WITH_PASSPHRASE
    );
    assert_ne!(EVM_ACCOUNT_XPUB_WITH_PASSPHRASE, EVM_ACCOUNT_XPUB);
    assert_ne!(TRON_ACCOUNT_XPUB_WITH_PASSPHRASE, TRON_ACCOUNT_XPUB);
}

/// `from_seed` got the identical one-line `Hint::Legacy` fix as
/// `from_mnemonic`, at a second, independent call site. Nothing calls
/// `from_seed` today, so nothing else would notice if that call site's
/// fix were ever reverted on its own - this exists so that regression is
/// caught here rather than never.
#[test]
fn from_seed_also_exports_the_legacy_prefixed_xpub() {
    let mnemonic = Mnemonic::<English>::new_from_phrase(TEST_MNEMONIC).unwrap();
    let seed = mnemonic.to_seed(Some("")).unwrap();

    let wallet = HdWallet::from_seed(&seed).unwrap();
    assert_eq!(
        wallet.account_xpub_string_for(ChainFamily::Evm).unwrap(),
        EVM_ACCOUNT_XPUB
    );
    assert_eq!(
        wallet.account_xpub_string_for(ChainFamily::Tron).unwrap(),
        TRON_ACCOUNT_XPUB
    );
}

/// The claim the `Hint::Legacy` fix rests on - that the encoding hint
/// changes only the base58 version bytes and nothing about which key,
/// and therefore which address, a path derives to - checked directly
/// instead of trusted from a comment. Reconstructs the pre-fix root key
/// (the implicit `Hint::SegWit` `root_from_seed(seed, None)` produced)
/// and the post-fix one side by side, and derives the same paths from
/// both.
#[test]
fn the_encoding_hint_does_not_change_which_addresses_are_derived() {
    let mnemonic = Mnemonic::<English>::new_from_phrase(TEST_MNEMONIC).unwrap();
    let seed = mnemonic.to_seed(Some("")).unwrap();

    let pre_fix_root = XPriv::root_from_seed(&seed[..], None).unwrap();
    let post_fix_root = XPriv::root_from_seed(&seed[..], Some(Hint::Legacy)).unwrap();

    for family in [ChainFamily::Evm, ChainFamily::Tron] {
        for index in 0..3u32 {
            let path = DerivationPath::from_str(&family.derivation_path(index)).unwrap();
            let pre_fix_key = pre_fix_root.derive_path(&path).unwrap();
            let post_fix_key = post_fix_root.derive_path(&path).unwrap();
            assert_eq!(
                public_key_to_address(&pre_fix_key.verify_key()),
                public_key_to_address(&post_fix_key.verify_key()),
                "hint changed the address derived for {family:?} index {index}"
            );
        }
    }
}
