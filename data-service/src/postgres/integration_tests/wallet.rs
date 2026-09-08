//! Account wallet repository behaviour against a real database (RCS-234).
//!
//! The invariant under test throughout: one xpub has one derivation counter,
//! and no two allocations from it ever return the same index.

use std::collections::HashSet;

use sqlx::Row;
use types::{StorePaymentMethodReader, StorePaymentMethodWriter};
use uuid::Uuid;

use super::super::PgDataService;
use super::super::tests::create_test_service;
use crate::{WalletReader, WalletWriter};

const XPUB_A: &str = "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuq\
                      pK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt";
const XPUB_B: &str = "xpub6D4BDPcP2GT577Vvch3R8wDkScZWzQzMMUm3PWbmWvVJrZwQY4VUNgqFJPMM3No\
                      2dFDFGTsxxpG5uJh7n7epu4trkrX7x7DogT5Uv6fcLW5";

/// Seed a user and return its id.
async fn seed_user(service: &PgDataService) -> Uuid {
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(user_id)
    .execute(service.pool())
    .await
    .expect("seed user");
    user_id
}

/// Seed a store owned by `user_id`.
async fn seed_store_for(service: &PgDataService, user_id: Uuid) -> Uuid {
    let store_id = Uuid::new_v4();
    sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(store_id)
        .bind(format!("store-{store_id}"))
        .bind(user_id)
        .execute(service.pool())
        .await
        .expect("seed store");
    store_id
}

// =========================================================================
// The invariant
// =========================================================================

/// Two payment methods configured with the same xpub must share one counter.
///
/// This is the bug RCS-234 exists to remove, in the shape it actually occurs:
/// not two merchants, but one store accepting ETH and USDC on the same key.
/// Under the old schema each row counted independently and both returned 0,
/// then 1, then 2 - the same three addresses, twice.
#[tokio::test]
#[ignore]
async fn two_methods_on_one_xpub_never_get_the_same_index() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;
    let store = seed_store_for(&service, user).await;

    let eth = StorePaymentMethodWriter::create_payment_method(
        &service, store, 1, None, "ETH", 18, XPUB_A,
    )
    .await
    .expect("create eth method");
    let usdc = StorePaymentMethodWriter::create_payment_method(
        &service,
        store,
        1,
        Some("0x1111111111111111111111111111111111111111"),
        "USDC",
        6,
        XPUB_A,
    )
    .await
    .expect("create usdc method");

    assert_eq!(
        eth.wallet_id, usdc.wallet_id,
        "the same xpub must resolve to the same wallet, or it has two counters"
    );

    let mut seen = HashSet::new();
    for _ in 0..4 {
        for method in [eth.id, usdc.id] {
            let index = StorePaymentMethodWriter::next_derivation_index(&service, method)
                .await
                .expect("allocate index");
            assert!(
                seen.insert(index),
                "index {index} was issued twice across methods sharing one \
                 xpub - that is two customers sent to one address"
            );
        }
    }
    assert_eq!(seen.len(), 8);
}

/// Concurrent allocation on one wallet must hand out distinct indices.
///
/// `next_derivation_index` is a single `UPDATE ... RETURNING`, so the row lock
/// serialises callers. A read-then-write would fail this under READ COMMITTED:
/// both readers see the same value and both return it.
#[tokio::test]
#[ignore]
async fn concurrent_allocation_on_one_wallet_issues_distinct_indices() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;
    let wallet = WalletWriter::create_wallet(&service, user, XPUB_A, None)
        .await
        .expect("create wallet");

    let service = std::sync::Arc::new(service);
    const N: usize = 32;

    let mut handles = Vec::with_capacity(N);
    for _ in 0..N {
        let svc = service.clone();
        let id = wallet.id;
        handles.push(tokio::spawn(async move {
            WalletWriter::next_derivation_index(&*svc, id).await
        }));
    }

    let mut indices = HashSet::new();
    for h in handles {
        let index = h.await.expect("join").expect("allocate");
        assert!(
            indices.insert(index),
            "index {index} was handed out twice under concurrency"
        );
    }

    assert_eq!(indices.len(), N, "every allocation must be distinct");
    assert_eq!(
        indices.iter().copied().max().unwrap(),
        N as i32 - 1,
        "indices must be contiguous from 0: a gap means an allocation was lost, \
         a duplicate means one was reused"
    );

    let after = WalletReader::get_wallet(&*service, wallet.id)
        .await
        .expect("read back")
        .expect("wallet exists");
    assert_eq!(
        after.derivation_index, N as i32,
        "the stored counter must be one past the last index issued"
    );
}

/// Re-adding an xpub the account already holds returns the same wallet.
#[tokio::test]
#[ignore]
async fn adding_a_known_xpub_does_not_create_a_second_counter() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;

    let first = WalletWriter::create_wallet(&service, user, XPUB_A, Some("first"))
        .await
        .expect("create");
    WalletWriter::next_derivation_index(&service, first.id)
        .await
        .expect("allocate");

    let again = WalletWriter::create_wallet(&service, user, XPUB_A, Some("again"))
        .await
        .expect("re-add");

    assert_eq!(
        first.id, again.id,
        "a second row for a key the account already holds would be a second \
         counter on it"
    );
    assert_eq!(
        again.derivation_index, 1,
        "re-adding must not rewind the counter - that re-issues address 0"
    );
    assert_eq!(
        WalletReader::list_wallets(&service, user)
            .await
            .unwrap()
            .len(),
        1
    );
}

// =========================================================================
// Store resolution
// =========================================================================

/// A store with no override derives from the account primary; one with an
/// override derives from that instead; clearing it falls back again.
#[tokio::test]
#[ignore]
async fn a_store_without_an_override_uses_the_account_primary() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;
    let store = seed_store_for(&service, user).await;

    assert!(
        WalletReader::resolve_store_wallet(&service, store)
            .await
            .unwrap()
            .is_none(),
        "an account with no wallet resolves to nothing, not to a default"
    );

    // The first wallet on an account becomes its primary.
    let primary = WalletWriter::create_wallet(&service, user, XPUB_A, Some("primary"))
        .await
        .unwrap();
    assert!(primary.is_primary);

    let resolved = WalletReader::resolve_store_wallet(&service, store)
        .await
        .unwrap()
        .expect("resolves to the primary");
    assert_eq!(resolved.id, primary.id);

    // Pin the store elsewhere.
    let other = WalletWriter::create_wallet(&service, user, XPUB_B, Some("other"))
        .await
        .unwrap();
    assert!(!other.is_primary, "only the first wallet is primary");

    WalletWriter::set_store_wallet(&service, store, other.id)
        .await
        .unwrap();
    assert_eq!(
        WalletReader::resolve_store_wallet(&service, store)
            .await
            .unwrap()
            .unwrap()
            .id,
        other.id,
        "an override must win over the primary"
    );

    // Moving the primary must not move an overridden store.
    WalletWriter::set_primary_wallet(&service, user, other.id)
        .await
        .unwrap();
    WalletWriter::set_store_wallet(&service, store, primary.id)
        .await
        .unwrap();
    assert_eq!(
        WalletReader::resolve_store_wallet(&service, store)
            .await
            .unwrap()
            .unwrap()
            .id,
        primary.id
    );

    // And clearing it falls back to whatever the primary now is.
    WalletWriter::clear_store_wallet(&service, store)
        .await
        .unwrap();
    assert_eq!(
        WalletReader::resolve_store_wallet(&service, store)
            .await
            .unwrap()
            .unwrap()
            .id,
        other.id,
        "with no override the store follows the current primary"
    );
}

/// A wallet on another account may not be pinned to this one's store.
#[tokio::test]
#[ignore]
async fn a_store_cannot_be_pinned_to_another_accounts_wallet() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let mine = seed_user(&service).await;
    let theirs = seed_user(&service).await;
    let store = seed_store_for(&service, mine).await;

    let not_mine = WalletWriter::create_wallet(&service, theirs, XPUB_A, None)
        .await
        .unwrap();

    assert!(
        WalletWriter::set_store_wallet(&service, store, not_mine.id)
            .await
            .is_err(),
        "pinning a store to another account's key would send this merchant's \
         money to that one"
    );
}

// =========================================================================
// Primary and deletion
// =========================================================================

/// Promotion demotes the outgoing primary, in one transaction.
#[tokio::test]
#[ignore]
async fn promoting_a_wallet_demotes_the_previous_primary() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;

    let a = WalletWriter::create_wallet(&service, user, XPUB_A, None)
        .await
        .unwrap();
    let b = WalletWriter::create_wallet(&service, user, XPUB_B, None)
        .await
        .unwrap();

    let promoted = WalletWriter::set_primary_wallet(&service, user, b.id)
        .await
        .unwrap();
    assert!(promoted.is_primary);

    assert_eq!(
        WalletReader::get_primary_wallet(&service, user)
            .await
            .unwrap()
            .unwrap()
            .id,
        b.id
    );
    assert!(
        !WalletReader::get_wallet(&service, a.id)
            .await
            .unwrap()
            .unwrap()
            .is_primary,
        "the outgoing primary must be demoted, not left as a second one"
    );
}

/// A wallet still in use cannot be deleted out from under the addresses it
/// derived.
#[tokio::test]
#[ignore]
async fn a_wallet_in_use_cannot_be_deleted() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;
    let store = seed_store_for(&service, user).await;

    let method = StorePaymentMethodWriter::create_payment_method(
        &service, store, 1, None, "ETH", 18, XPUB_A,
    )
    .await
    .unwrap();

    assert!(
        WalletWriter::delete_wallet(&service, method.wallet_id)
            .await
            .is_err(),
        "deleting a wallet a payment method still derives from would strand \
         every address it has issued"
    );

    StorePaymentMethodWriter::delete_payment_method(&service, method.id)
        .await
        .unwrap();
    WalletWriter::delete_wallet(&service, method.wallet_id)
        .await
        .expect("deletable once nothing references it");
}

/// Rotation repoints at the new key's wallet and does not rewind a counter.
#[tokio::test]
#[ignore]
async fn rotation_repoints_without_resetting_the_counter() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;
    let store = seed_store_for(&service, user).await;

    let method = StorePaymentMethodWriter::create_payment_method(
        &service, store, 1, None, "ETH", 18, XPUB_A,
    )
    .await
    .unwrap();
    for _ in 0..3 {
        StorePaymentMethodWriter::next_derivation_index(&service, method.id)
            .await
            .unwrap();
    }

    // The account has used XPUB_B before and it is already at index 7.
    let b = WalletWriter::create_wallet(&service, user, XPUB_B, None)
        .await
        .unwrap();
    for _ in 0..7 {
        WalletWriter::next_derivation_index(&service, b.id)
            .await
            .unwrap();
    }

    let rotation = service
        .rotate_payment_method_xpub(store, method.id, XPUB_B, Some("test"))
        .await
        .expect("rotate");
    assert_eq!(rotation.previous_xpub, XPUB_A);
    assert_eq!(rotation.previous_derivation_index, 3);

    let after = StorePaymentMethodReader::get_payment_method(&service, method.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.wallet_id, b.id,
        "the method must point at the new key"
    );
    assert_eq!(
        after.derivation_index, 7,
        "rotation must not reset a shared counter to zero - that re-issues \
         every address the key has already produced"
    );
}

/// The derivation index reaches the payment option that used it.
#[tokio::test]
#[ignore]
async fn payment_options_record_the_wallet_and_index_they_used() {
    let Some(service) = create_test_service().await else {
        return;
    };
    let user = seed_user(&service).await;
    let store = seed_store_for(&service, user).await;

    let method = StorePaymentMethodWriter::create_payment_method(
        &service, store, 1, None, "ETH", 18, XPUB_A,
    )
    .await
    .unwrap();

    let invoice_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO invoices (id, store_id, currency, amount, expires_at) \
         VALUES ($1, $2, 'USD', 100, NOW() + interval '1 hour')",
    )
    .bind(&invoice_id)
    .bind(store)
    .execute(service.pool())
    .await
    .unwrap();

    let index = StorePaymentMethodWriter::next_derivation_index(&service, method.id)
        .await
        .unwrap();

    let option = types::PaymentOptionData {
        id: types::PaymentOptionId::new(),
        invoice_id: types::InvoiceId::from_string(invoice_id.clone()),
        payment_method_id: types::PaymentMethodId::new("ETH", 1),
        chain_id: 1,
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: format!("0x{:040x}", Uuid::new_v4().as_u128()),
        wallet_id: Some(method.wallet_id),
        derivation_index: Some(index),
        amount: "1".to_string(),
        rate: None,
        rate_at: None,
        is_active: true,
        created_at: chrono::Utc::now(),
    };
    types::PaymentOptionWriter::create(&service, &option)
        .await
        .unwrap();

    let read = types::PaymentOptionReader::get(&service, &option.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.wallet_id, Some(method.wallet_id));
    assert_eq!(
        read.derivation_index,
        Some(index),
        "without the index the address cannot be tied back to a position on \
         the key"
    );

    let stored: Option<i32> =
        sqlx::query("SELECT derivation_index FROM payment_options WHERE id = $1")
            .bind(option.id.0)
            .fetch_one(service.pool())
            .await
            .unwrap()
            .get("derivation_index");
    assert_eq!(stored, Some(index));
}
