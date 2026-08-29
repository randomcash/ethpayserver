//! Payout repository integration tests.

use chrono::Utc;
use types::{PayoutData, PayoutStatus, StoreId};
use uuid::Uuid;

use crate::{PayoutReader, PayoutWriter};

use super::super::PgDataService;
use super::{create_test_service, seed_store};

/// Seed the store a payout's foreign key requires and return a pending payout
/// bound to it.
///
/// `payouts.store_id` REFERENCES stores(id), so a bare `StoreId::new()`
/// violates `payouts_store_id_fkey`.
async fn seeded_test_payout(service: &PgDataService) -> PayoutData {
    let store_id = seed_store(service).await;

    PayoutData {
        id: Uuid::new_v4(),
        store_id,
        invoice_ids: vec!["inv_1".to_string(), "inv_2".to_string()],
        destination_address: "0x1111111111111111111111111111111111111111".to_string(),
        chain_id: 11155111,
        asset_type: "native".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        amount: "3000000000000000000".to_string(),
        tx_hash: None,
        status: PayoutStatus::Pending,
        fee_amount: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    }
}

#[tokio::test]
#[ignore]
async fn integration_payout_create_and_read() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .expect("payout was just created");

    assert_eq!(fetched.id, payout.id);
    assert_eq!(fetched.store_id, payout.store_id);
    assert_eq!(fetched.destination_address, payout.destination_address);
    assert_eq!(fetched.chain_id, payout.chain_id);
    assert_eq!(fetched.asset_type, "native");
    assert_eq!(fetched.asset_symbol, "ETH");
    assert_eq!(fetched.amount, payout.amount);
    assert_eq!(fetched.status, PayoutStatus::Pending);
    assert!(fetched.tx_hash.is_none());
    assert!(fetched.confirmed_at.is_none());
}

/// `invoice_ids` is a JSONB column, so the list has to survive a
/// serialise/deserialise round trip with its order intact.
#[tokio::test]
#[ignore]
async fn integration_payout_invoice_ids_round_trip_through_jsonb() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.invoice_ids, vec!["inv_1", "inv_2"]);
}

#[tokio::test]
#[ignore]
async fn integration_payout_empty_invoice_ids_round_trip() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = PayoutData {
        invoice_ids: vec![],
        ..seeded_test_payout(&service).await
    };
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .unwrap();

    assert!(fetched.invoice_ids.is_empty());
}

#[tokio::test]
#[ignore]
async fn integration_payout_erc20_keeps_its_token_address() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let usdc = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
    let payout = PayoutData {
        asset_type: "erc20".to_string(),
        asset_symbol: "USDC".to_string(),
        token_address: Some(usdc.to_string()),
        amount: "5000000".to_string(),
        ..seeded_test_payout(&service).await
    };
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.asset_type, "erc20");
    assert_eq!(fetched.asset_symbol, "USDC");
    assert_eq!(fetched.token_address.as_deref(), Some(usdc));
    assert_eq!(fetched.amount, "5000000");
}

#[tokio::test]
#[ignore]
async fn integration_payout_get_unknown_id_returns_none() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let fetched = PayoutReader::get_payout(&service, Uuid::new_v4())
        .await
        .unwrap();

    assert!(fetched.is_none());
}

#[tokio::test]
#[ignore]
async fn integration_payout_update_status_sets_tx_hash_and_fee() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    PayoutWriter::update_payout_status(
        &service,
        payout.id,
        PayoutStatus::Broadcasting,
        Some("0xpayouttx"),
        Some("42000"),
        None,
    )
    .await
    .unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.status, PayoutStatus::Broadcasting);
    assert_eq!(fetched.tx_hash.as_deref(), Some("0xpayouttx"));
    assert_eq!(fetched.fee_amount.as_deref(), Some("42000"));
    assert!(!fetched.status.is_final());
}

/// `update_payout_status` COALESCEs tx_hash/fee/error, so a later update that
/// passes `None` must not wipe the hash recorded at broadcast time.
#[tokio::test]
#[ignore]
async fn integration_payout_update_status_preserves_existing_tx_hash() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    PayoutWriter::update_payout_status(
        &service,
        payout.id,
        PayoutStatus::Broadcasting,
        Some("0xpayouttx"),
        None,
        None,
    )
    .await
    .unwrap();

    PayoutWriter::update_payout_status(
        &service,
        payout.id,
        PayoutStatus::Failed,
        None,
        None,
        Some("gas estimation failed"),
    )
    .await
    .unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.status, PayoutStatus::Failed);
    assert_eq!(fetched.tx_hash.as_deref(), Some("0xpayouttx"));
    assert_eq!(
        fetched.error_message.as_deref(),
        Some("gas estimation failed")
    );
}

#[tokio::test]
#[ignore]
async fn integration_payout_confirm_sets_confirmed_at() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    PayoutWriter::confirm_payout(&service, payout.id)
        .await
        .unwrap();

    let fetched = PayoutReader::get_payout(&service, payout.id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.status, PayoutStatus::Confirmed);
    assert!(fetched.status.is_final());
    assert!(fetched.confirmed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn integration_payout_get_for_store_is_scoped_and_counted() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    let (total, payouts) = PayoutReader::get_payouts_for_store(&service, payout.store_id, 50, 0)
        .await
        .unwrap();

    assert_eq!(total, 1);
    assert_eq!(payouts.len(), 1);
    assert_eq!(payouts[0].id, payout.id);

    // A store with no payouts must not see another store's rows.
    let (other_total, other_payouts) =
        PayoutReader::get_payouts_for_store(&service, StoreId::new(), 50, 0)
            .await
            .unwrap();

    assert_eq!(other_total, 0);
    assert!(other_payouts.is_empty());
}

#[tokio::test]
#[ignore]
async fn integration_payout_get_for_store_paginates() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let first = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &first).await.unwrap();
    let second = PayoutData {
        id: Uuid::new_v4(),
        ..first.clone()
    };
    PayoutWriter::create_payout(&service, &second).await.unwrap();

    let (total, page) = PayoutReader::get_payouts_for_store(&service, first.store_id, 1, 0)
        .await
        .unwrap();

    // The count reflects every payout for the store, the page only the limit.
    assert_eq!(total, 2);
    assert_eq!(page.len(), 1);

    let (_, second_page) = PayoutReader::get_payouts_for_store(&service, first.store_id, 1, 1)
        .await
        .unwrap();

    assert_eq!(second_page.len(), 1);
    assert_ne!(second_page[0].id, page[0].id);
}

/// The payout broadcaster picks up work via `get_active_payouts`, so a payout
/// must appear there while pending and drop out once it reaches a final state.
#[tokio::test]
#[ignore]
async fn integration_payout_active_excludes_final_statuses() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let payout = seeded_test_payout(&service).await;
    PayoutWriter::create_payout(&service, &payout).await.unwrap();

    let active = PayoutReader::get_active_payouts(&service).await.unwrap();
    assert!(active.iter().any(|p| p.id == payout.id));

    PayoutWriter::confirm_payout(&service, payout.id)
        .await
        .unwrap();

    let active = PayoutReader::get_active_payouts(&service).await.unwrap();
    assert!(!active.iter().any(|p| p.id == payout.id));
}
