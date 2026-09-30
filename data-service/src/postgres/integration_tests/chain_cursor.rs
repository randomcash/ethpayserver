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
async fn a_lagging_commit_cannot_move_a_cursor_backwards_within_an_epoch() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());
    let chain_id = 999_888_775u64;
    let at = |epoch, seq| ChainCursor {
        epoch,
        seq,
        block_height: seq,
    };

    service
        .commit_chain_cursor(&adapter_id, chain_id, at(1, 10))
        .await
        .unwrap();
    // A slower instance committing an older position must not win.
    service
        .commit_chain_cursor(&adapter_id, chain_id, at(1, 5))
        .await
        .unwrap();
    let cursors = service.chain_cursors(&adapter_id).await.unwrap();
    assert_eq!(cursors.get(&chain_id), Some(&at(1, 10)));

    // A new epoch restarts seq, so a lower seq there is legitimate.
    service
        .commit_chain_cursor(&adapter_id, chain_id, at(2, 1))
        .await
        .unwrap();
    let cursors = service.chain_cursors(&adapter_id).await.unwrap();
    assert_eq!(cursors.get(&chain_id), Some(&at(2, 1)));
}

#[tokio::test]
#[ignore]
async fn deleting_a_cursor_removes_only_that_chain_and_adapter() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());
    let other_adapter_id = format!("test-adapter-{}", uuid::Uuid::new_v4());
    let (gone, kept) = (999_888_771u64, 999_888_772u64);
    let cursor = ChainCursor {
        epoch: 3,
        seq: 7,
        block_height: 70,
    };

    for (adapter, chain) in [
        (&adapter_id, gone),
        (&adapter_id, kept),
        (&other_adapter_id, gone),
    ] {
        service
            .commit_chain_cursor(adapter, chain, cursor)
            .await
            .unwrap();
    }
    service
        .delete_chain_cursor(&adapter_id, gone)
        .await
        .unwrap();
    // Deleting an absent row is not an error.
    service
        .delete_chain_cursor(&adapter_id, gone)
        .await
        .unwrap();

    let cursors = service.chain_cursors(&adapter_id).await.unwrap();
    assert_eq!(cursors.get(&gone), None);
    assert_eq!(cursors.get(&kept), Some(&cursor));
    let other = service.chain_cursors(&other_adapter_id).await.unwrap();
    assert_eq!(other.get(&gone), Some(&cursor));
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

/// A watch that expired (is inactive) is invisible to both the re-arm reset
/// and the payment-option lookup the apply path uses. Recovering a payment that
/// confirmed on an expired watch therefore cannot rely on either; this pins
/// that boundary so a change to it is a decision, not an accident.
#[tokio::test]
#[ignore]
async fn inactive_watch_is_neither_rearmed_nor_resolved_to_a_payment_option() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let chain = ChainId::evm(1);
    let po = test_payment_option(&invoice.id, &chain);
    PaymentOptionWriter::create(&service, &po).await.unwrap();

    let address = unique_address();
    WatchedAddressWriter::upsert(&service, &address, &po.id, &chain, None)
        .await
        .unwrap();
    WatchedAddressWriter::mark_notified(&service, &address, &chain, None)
        .await
        .unwrap();

    // While active the lookup resolves.
    assert_eq!(
        WatchedAddressReader::get_payment_option_id(&service, &address, &chain, None)
            .await
            .unwrap(),
        Some(po.id.clone())
    );

    assert!(
        WatchedAddressWriter::deactivate(&service, &address, &chain, None)
            .await
            .unwrap()
    );

    service
        .reset_chain_watch_notifications(chain.evm_chain_id().unwrap())
        .await
        .unwrap();

    // Read the flag off the row itself: `get_pending` filters on `is_active`
    // too, so asserting through it would pass even if the reset re-armed the
    // inactive row.
    let notified: bool = sqlx::query_scalar(
        "SELECT monitor_notified FROM watched_addresses WHERE address = $1 AND chain_id = $2",
    )
    .bind(&address)
    .bind(chain.as_str())
    .fetch_one(service.pool())
    .await
    .unwrap();
    assert!(
        notified,
        "an inactive watch must not be re-armed by the reset"
    );
    assert_eq!(
        WatchedAddressReader::get_payment_option_id(&service, &address, &chain, None)
            .await
            .unwrap(),
        None,
        "an inactive watch must not resolve to a payment option"
    );
}
