//! Durable event-resume cursor integration tests.

use types::{
    ChainId, InvoiceWriter, PaymentOptionWriter, WatchedAddressReader, WatchedAddressWriter,
};

use crate::chain_cursor::{ChainCursor, ChainCursorReader, ChainCursorWriter};

use super::{create_test_service, seeded_test_invoice, test_payment_option, unique_address};

#[tokio::test]
#[ignore]
async fn a_chain_with_no_cursor_is_absent_not_zero() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());

    let cursors = service.chain_cursors(&adapter_id).await.unwrap();
    assert!(cursors.is_empty());
}

#[tokio::test]
#[ignore]
async fn committing_a_cursor_makes_it_readable_and_scoped_to_its_adapter() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());
    let other_adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());
    let chain_id = 999_888_777u64;

    let cursor = ChainCursor {
        epoch: 1,
        seq: 42,
        block_height: 100,
    };
    service
        .commit_chain_cursor(&adapter_id, chain_id, cursor)
        .await
        .unwrap();

    let cursors = service.chain_cursors(&adapter_id).await.unwrap();
    assert_eq!(cursors.get(&chain_id), Some(&cursor));

    // A different adapter_id must not see it - two adapters must never be
    // able to resume from each other's progress.
    let other = service.chain_cursors(&other_adapter_id).await.unwrap();
    assert!(other.is_empty());
}

#[tokio::test]
#[ignore]
async fn committing_again_replaces_rather_than_duplicates() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());
    let chain_id = 999_888_776u64;

    service
        .commit_chain_cursor(
            &adapter_id,
            chain_id,
            ChainCursor {
                epoch: 1,
                seq: 1,
                block_height: 10,
            },
        )
        .await
        .unwrap();
    service
        .commit_chain_cursor(
            &adapter_id,
            chain_id,
            ChainCursor {
                epoch: 1,
                seq: 2,
                block_height: 20,
            },
        )
        .await
        .unwrap();

    let cursors = service.chain_cursors(&adapter_id).await.unwrap();
    assert_eq!(cursors.len(), 1);
    assert_eq!(
        cursors.get(&chain_id),
        Some(&ChainCursor {
            epoch: 1,
            seq: 2,
            block_height: 20,
        })
    );
}

#[tokio::test]
#[ignore]
async fn resetting_watch_notifications_only_touches_the_named_chain() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let target_chain = ChainId::evm(1);
    let other_chain = ChainId::evm(137);

    let po_target = test_payment_option(&invoice.id, &target_chain);
    let po_other = test_payment_option(&invoice.id, &other_chain);
    PaymentOptionWriter::create(&service, &po_target)
        .await
        .unwrap();
    PaymentOptionWriter::create(&service, &po_other)
        .await
        .unwrap();

    let target_address = unique_address();
    let other_address = unique_address();
    WatchedAddressWriter::upsert(
        &service,
        &target_address,
        &po_target.id,
        &target_chain,
        None,
    )
    .await
    .unwrap();
    WatchedAddressWriter::upsert(&service, &other_address, &po_other.id, &other_chain, None)
        .await
        .unwrap();

    // Both start pending (`monitor_notified = FALSE` by default); mark both
    // notified so the reset below has something to actually undo.
    WatchedAddressWriter::mark_notified(&service, &target_address, &target_chain, None)
        .await
        .unwrap();
    WatchedAddressWriter::mark_notified(&service, &other_address, &other_chain, None)
        .await
        .unwrap();

    let pending_before = WatchedAddressReader::get_pending(&service).await.unwrap();
    assert!(!pending_before.iter().any(|w| w.address == target_address));
    assert!(!pending_before.iter().any(|w| w.address == other_address));

    let affected = service
        .reset_chain_watch_notifications(target_chain.evm_chain_id().unwrap())
        .await
        .unwrap();
    assert!(affected >= 1);

    let pending_after = WatchedAddressReader::get_pending(&service).await.unwrap();
    assert!(
        pending_after.iter().any(|w| w.address == target_address),
        "the target chain's watch should be pending again"
    );
    assert!(
        !pending_after.iter().any(|w| w.address == other_address),
        "a different chain's watch must be left alone"
    );
}
