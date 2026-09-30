#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::PaymentDetected;
use evm::{Address, B256, U256};
use std::sync::Arc;
use types::ChainId;
use types::{
    InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentReader, StoreId, TokenData,
    TokenWriter,
};

use super::helpers::{MockEVMMonitor, create_test_consumer, create_test_invoice};
use crate::services::event_consumer::EventConsumer;

#[tokio::test]
async fn test_handle_payment_detected_native() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    // Create PaymentDetected event
    let event = PaymentDetected {
        chain_id: 1, // Ethereum mainnet
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(500000000000000000u64), // 0.5 ETH
        tx_hash: B256::ZERO,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    // Handle the event
    consumer.handle_payment_detected(event).await.unwrap();

    // Verify payment was created
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].asset_symbol, "ETH");
    assert_eq!(payments[0].amount, "500000000000000000");
    assert!(!payments[0].reorged);
}

#[tokio::test]
async fn test_handle_payment_detected_unknown_chain() {
    // With network-agnostic approach, payments from unknown chains are accepted
    // chain_id is stored on the payment, not the invoice
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());

    let consumer: EventConsumer<InMemoryDataService, MockEVMMonitor> = EventConsumer::new(
        bridge.clone(),
        ds.clone(),
        None,
        None,
        None,
        Arc::new(crate::services::email::NoopEmailSender),
    );

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();

    // Create network-agnostic invoice
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(),
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(&*ds, &invoice).await.unwrap();

    // Create PaymentDetected event with unknown chain
    let event = PaymentDetected {
        chain_id: 99999, // Unknown chain
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: U256::from(1000000u64),
        tx_hash: B256::ZERO,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::ZERO,
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    // Should succeed - unknown chains are now accepted
    consumer.handle_payment_detected(event).await.unwrap();

    // Verify payment was created with chain_id
    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].chain_id, ChainId::evm(99999));
    assert_eq!(payments[0].asset_symbol, "ETH"); // Fallback for unknown chains
}

/// A batching contract, a multicall, or an exchange sweep can settle
/// two transfers to two watched addresses in a single transaction. Both must
/// be recorded - before the fix, `payments` had no way to tell them apart
/// (`unique_payment_tx` was `(tx_hash, chain_id)` alone), so the second
/// `PaymentDetected` event's upsert silently overwrote the first instead of
/// inserting a second row.
#[tokio::test]
async fn test_handle_payment_detected_two_transfers_in_one_tx_both_survive() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    let shared_tx_hash = B256::repeat_byte(0xcd);

    let first_amount = U256::from(500000000000000000u64); // 0.5 ETH
    let second_amount = U256::from(300000000000000000u64); // 0.3 ETH

    let base_event = PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::ZERO,
        amount: first_amount,
        tx_hash: shared_tx_hash,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: Some(0),
        // ERC20, not native: this is the batched-transfer case - a multicall
        // or an exchange sweep emitting two Transfer logs in one transaction.
        // Two *native* transfers in one transaction cannot happen, because
        // `check_native_payments` reads each transaction's top-level
        // `to`/`value`, one entry per hash, so a native event's log index is
        // meaningless and the fixture used to lean on one that production
        // never produces.
        is_native: false,
        token_address: Some(Address::repeat_byte(0xde)),
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    let first_event = base_event.clone();
    let second_event = PaymentDetected {
        payment_address: Address::repeat_byte(0x02),
        amount: second_amount,
        log_index: Some(1),
        from_address: Address::repeat_byte(0xef),
        ..base_event
    };

    consumer.handle_payment_detected(first_event).await.unwrap();
    consumer
        .handle_payment_detected(second_event)
        .await
        .unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(
        payments.len(),
        2,
        "two transfers sharing a tx_hash must produce two payments, not one \
         overwritten by the other"
    );

    let amounts: std::collections::HashSet<_> = payments.iter().map(|p| p.amount.clone()).collect();
    assert!(amounts.contains(&first_amount.to_string()));
    assert!(amounts.contains(&second_amount.to_string()));
}

/// A native transfer (`log_index: None`) and an ERC20
/// transfer whose *real* log index happens to be 0 can share a `tx_hash` -
/// e.g. a contract that receives ETH directly at the top level and, in the
/// same transaction, emits a Transfer log at index 0 to a different watched
/// address. If native transfers were keyed on `tx_index = 0`, this would
/// collide with exactly that ERC20 transfer and silently overwrite it. The
/// fix keys native transfers on a -1 sentinel instead, which no real log
/// index can ever produce.
#[tokio::test]
async fn test_handle_payment_detected_native_and_log_index_zero_both_survive() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    let shared_tx_hash = B256::repeat_byte(0xef);

    let native_amount = U256::from(500000000000000000u64); // 0.5 ETH
    let erc20_amount = U256::from(1_000_000u64);

    let native_event = PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::repeat_byte(0x01),
        amount: native_amount,
        tx_hash: shared_tx_hash,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: None,
        is_native: true,
        token_address: None,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    let erc20_event = PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::repeat_byte(0x02),
        amount: erc20_amount,
        tx_hash: shared_tx_hash,
        block_number: 12345678,
        block_hash: B256::ZERO,
        log_index: Some(0),
        is_native: false,
        token_address: Some(Address::repeat_byte(0x03)),
        from_address: Address::repeat_byte(0xef),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    consumer
        .handle_payment_detected(native_event)
        .await
        .unwrap();
    consumer.handle_payment_detected(erc20_event).await.unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(
        payments.len(),
        2,
        "a native transfer and an ERC20 transfer at log_index 0 sharing a \
         tx_hash must both survive, not collide on tx_index"
    );

    let amounts: std::collections::HashSet<_> = payments.iter().map(|p| p.amount.clone()).collect();
    assert!(amounts.contains(&native_amount.to_string()));
    assert!(amounts.contains(&erc20_amount.to_string()));
}

/// Same chain, address, symbol and decimals as the zkSync Era USDC row the
/// tokens-seed migration inserts. This is the entry point a real payment
/// actually takes - `handle_payment_detected` calling `TokenReader::
/// get_by_address` - rather than a query against `tokens` on its own, so a
/// seeded row that only matched its own checksummed casing back would show up
/// here as the "ERC20" fallback instead of "USDC".
#[tokio::test]
async fn test_handle_payment_detected_erc20_resolves_seeded_token_symbol() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());

    let invoice_id = InvoiceId::new();
    let store_id = StoreId::new();
    create_test_invoice(&ds, &invoice_id, store_id).await;

    let zksync_era = ChainId::evm(324);
    let checksummed_usdc = "0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4";
    TokenWriter::insert(
        &*ds,
        &TokenData::new("erc20", checksummed_usdc, zksync_era)
            .with_symbol("USDC")
            .with_decimals(6),
    )
    .await
    .unwrap();

    let event = PaymentDetected {
        chain_id: 324,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: Address::repeat_byte(0x09),
        amount: U256::from(1_000_000u64), // 1 USDC, 6 decimals
        tx_hash: B256::repeat_byte(0x11),
        block_number: 1,
        block_hash: B256::ZERO,
        log_index: Some(0),
        is_native: false,
        token_address: Some(checksummed_usdc.parse().unwrap()),
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    };

    consumer.handle_payment_detected(event).await.unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(
        payments[0].asset_symbol, "USDC",
        "a seeded token must resolve to its symbol, not the ERC20 fallback \
         recorded for a contract the tokens table has no row for"
    );
}

/// An invoice with one payment option whose watch has then expired.
async fn expired_watch_fixture(
    ds: &InMemoryDataService,
    token: Option<&str>,
) -> (InvoiceId, types::PaymentOptionId, Address) {
    use types::{
        InvoiceWriter, PaymentMethodId, PaymentOptionData, PaymentOptionId, PaymentOptionWriter,
        WatchedAddressWriter,
    };

    let invoice_id = InvoiceId::new();
    InvoiceWriter::upsert(
        ds,
        &InvoiceData {
            id: invoice_id.clone(),
            store_id: StoreId::new(),
            currency: "ETH".to_string(),
            status: InvoiceStatus::Expired,
            amount: "1".to_string(),
            amount_received: "0".to_string(),
            created_at: Utc::now() - chrono::Duration::days(40),
            expires_at: Utc::now() - chrono::Duration::days(30),
            metadata: None,
            customer_email: None,
            extra: None,
        },
    )
    .await
    .unwrap();

    let chain = ChainId::parse("eip155:1").unwrap();
    let address = Address::repeat_byte(0x42);
    let address_str = format!("{:#x}", address);
    let po = PaymentOptionData {
        id: PaymentOptionId(uuid::Uuid::new_v4()),
        invoice_id: invoice_id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", &chain),
        chain_id: chain.clone(),
        asset_symbol: "ETH".to_string(),
        token_address: token.map(str::to_string),
        decimals: 18,
        payment_address: address_str.clone(),
        wallet_id: None,
        derivation_index: None,
        amount: "1".to_string(),
        rate: None,
        rate_at: None,
        is_active: true,
        created_at: Utc::now(),
    };
    PaymentOptionWriter::create(ds, &po).await.unwrap();
    WatchedAddressWriter::upsert(ds, &address_str, &po.id, &chain, token)
        .await
        .unwrap();
    // The watch expires.
    WatchedAddressWriter::deactivate(ds, &address_str, &chain, token)
        .await
        .unwrap();
    (invoice_id, po.id, address)
}

fn detected(invoice_id: &InvoiceId, address: Address, token: Option<Address>) -> PaymentDetected {
    PaymentDetected {
        chain_id: 1,
        invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
        payment_address: address,
        amount: U256::from(1_000_000_000_000_000_000u64),
        tx_hash: B256::repeat_byte(0xcc),
        block_number: 1,
        block_hash: B256::ZERO,
        log_index: token.map(|_| 0),
        is_native: token.is_none(),
        token_address: token,
        from_address: Address::repeat_byte(0xab),
        confirmations: 1,
        required_confirmations: 12,
        detected_at: Utc::now(),
    }
}

/// A payment to an address whose watch has expired is on chain and belongs to
/// the invoice. It must be credited, and once confirmed it must settle the
/// expired invoice as late-paid so the merchant can see it.
///
/// The in-memory store does not run the database trigger that sums
/// `credited_amount` into `amount_received`, so the test applies that sum
/// itself: an uncredited payment leaves the invoice unpaid, exactly as in
/// production.
#[tokio::test]
async fn payment_to_an_expired_watch_is_credited_and_settles_late() {
    use types::{InvoiceReader, PaymentOptionReader};

    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());
    let (invoice_id, po_id, address) = expired_watch_fixture(&ds, None).await;
    // Sanity: the watch really is gone, so the fixture reproduces the bug.
    assert!(
        types::WatchedAddressReader::get_payment_option_id(
            &*ds,
            &format!("{:#x}", address),
            &ChainId::parse("eip155:1").unwrap(),
            None
        )
        .await
        .unwrap()
        .is_none()
    );
    let _ = PaymentOptionReader::get(&*ds, &po_id)
        .await
        .unwrap()
        .unwrap();

    let event = detected(&invoice_id, address, None);
    let (tx_hash, amount) = (event.tx_hash, event.amount);
    consumer.handle_payment_detected(event).await.unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].payment_option_id, Some(po_id.0));
    assert_eq!(payments[0].credited_amount.as_deref(), Some("1"));

    // Stand in for the trigger.
    let received: u64 = payments
        .iter()
        .filter_map(|p| p.credited_amount.as_deref()?.parse::<u64>().ok())
        .sum();
    types::InvoiceWriter::update_amount_received(&*ds, &invoice_id, &received.to_string())
        .await
        .unwrap();

    consumer
        .handle_payment_confirmed(evm::monitor::events::PaymentConfirmed {
            tx_index: -1,
            chain_id: 1,
            invoice_id: uuid::Uuid::parse_str(invoice_id.as_str()).unwrap(),
            payment_address: address,
            amount,
            tx_hash,
            block_number: 1,
            confirmations: 12,
            confirmed_at: Utc::now(),
        })
        .await
        .unwrap();

    let invoice = InvoiceReader::get(&*ds, &invoice_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.amount_received, "1");
    assert_eq!(invoice.status, InvoiceStatus::LatePaid);
}

/// The token branch of the fallback: an ERC-20 option matched by a
/// differently-cased address must still be found.
#[tokio::test]
async fn erc20_payment_to_an_expired_watch_is_credited() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());
    let token = Address::repeat_byte(0xAB);
    // Stored with an upper-case checksum-style token, as a merchant-side
    // writer might; the event side formats lower-case.
    let token_stored = format!("{:#x}", token)
        .to_uppercase()
        .replacen("0X", "0x", 1);
    let (invoice_id, po_id, address) = expired_watch_fixture(&ds, Some(&token_stored)).await;

    consumer
        .handle_payment_detected(detected(&invoice_id, address, Some(token)))
        .await
        .unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].payment_option_id, Some(po_id.0));
    assert_eq!(payments[0].credited_amount.as_deref(), Some("1"));
}

/// The fallback must not credit a payment that matches none of the invoice's
/// options: wrong address, or a token where the option is native.
#[tokio::test]
async fn payment_matching_no_option_of_the_invoice_stays_uncredited() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());
    let (invoice_id, _po_id, address) = expired_watch_fixture(&ds, None).await;

    consumer
        .handle_payment_detected(detected(&invoice_id, Address::repeat_byte(0x99), None))
        .await
        .unwrap();
    let mut token_event = detected(&invoice_id, address, Some(Address::repeat_byte(0xAB)));
    token_event.tx_hash = B256::repeat_byte(0xdd);
    consumer.handle_payment_detected(token_event).await.unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 2);
    for p in &payments {
        assert_eq!(p.payment_option_id, None);
        assert_eq!(p.credited_amount, None);
    }
}

/// A replay re-delivers transfers already applied. Applying the same
/// detection twice must leave one payment row with the amount credited once,
/// or a rescan would double-credit merchants.
#[tokio::test]
async fn reapplying_the_same_detection_credits_once() {
    let ds = Arc::new(InMemoryDataService::new());
    let bridge = Arc::new(MemoryBridge::new());
    let consumer = create_test_consumer(ds.clone(), bridge.clone());
    let (invoice_id, _po_id, address) = expired_watch_fixture(&ds, None).await;

    let event = detected(&invoice_id, address, None);
    consumer
        .handle_payment_detected(event.clone())
        .await
        .unwrap();
    consumer.handle_payment_detected(event).await.unwrap();

    let payments = PaymentReader::get_for_invoice(&*ds, &invoice_id)
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].credited_amount.as_deref(), Some("1"));
    // `amount_received` is the sum of `credited_amount` over the invoice's
    // payment rows (a database trigger in production), so assert that sum:
    // a second credit under a new row id would show up here.
    let total: u128 = payments
        .iter()
        .filter_map(|p| p.credited_amount.as_deref())
        .map(|c| c.parse::<u128>().unwrap())
        .sum();
    assert_eq!(total, 1);
}
