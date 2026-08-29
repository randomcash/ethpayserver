//! `do_create_invoice` — the tool an agent calls to request payment.

use types::{PaymentOptionReader, StoreId};
use uuid::Uuid;

use crate::server::CreateInvoiceArgs;
use crate::testkit::{StubRateProvider, TestHarness, parse_ok};

/// A USD/100.00 invoice request against the harness's store.
fn usd_args(store_id: StoreId) -> CreateInvoiceArgs {
    CreateInvoiceArgs {
        store_id: store_id.0.to_string(),
        currency: "USD".to_string(),
        amount: "100.00".to_string(),
        expiration_seconds: None,
        metadata: None,
        customer_email: None,
    }
}

#[tokio::test]
async fn creates_a_pending_invoice_with_a_converted_payment_option() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let json = parse_ok(h.server.do_create_invoice(usd_args(h.store_id)).await);

    assert_eq!(json["status"], "pending");
    assert_eq!(json["currency"], "USD");
    assert_eq!(json["amount"], "100.00");
    assert_eq!(json["amount_received"], "0");

    let options = json["payment_options"].as_array().unwrap();
    assert_eq!(options.len(), 1);
    let option = &options[0];
    assert_eq!(option["asset_symbol"], "ETH");
    assert_eq!(option["chain_id"], crate::testkit::CHAIN_ID);
    assert_eq!(option["decimals"], 18);
    assert_eq!(option["rate"], crate::testkit::USD_TO_ETH);
    assert_eq!(option["is_active"], true);
    // 100.00 USD * 0.0005 ETH/USD = 0.05 ETH = 5e16 wei
    assert_eq!(option["amount"], "50000000000000000");
    assert!(
        option["payment_address"]
            .as_str()
            .unwrap()
            .starts_with("0x"),
        "expected a derived EVM address, got {}",
        option["payment_address"]
    );
}

#[tokio::test]
async fn persists_the_invoice_and_its_payment_option() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let json = parse_ok(h.server.do_create_invoice(usd_args(h.store_id)).await);
    let invoice_id = types::InvoiceId::from_string(json["id"].as_str().unwrap().to_string());

    let stored = types::InvoiceReader::get(&*h.data, &invoice_id)
        .await
        .unwrap()
        .expect("invoice should be persisted");
    assert_eq!(stored.store_id, h.store_id);
    assert_eq!(stored.currency, "USD");

    let options = PaymentOptionReader::get_for_invoice(&*h.data, &invoice_id)
        .await
        .unwrap();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].amount, "50000000000000000");
}

#[tokio::test]
async fn rejects_a_store_outside_the_session_scope() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let err = h
        .server
        .do_create_invoice(usd_args(TestHarness::foreign_store_id()))
        .await
        .unwrap_err();

    assert!(err.starts_with("Unauthorized"), "got: {err}");
}

#[tokio::test]
async fn rejects_a_malformed_store_id() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = usd_args(h.store_id);
    args.store_id = "not-a-uuid".to_string();

    assert_eq!(
        h.server.do_create_invoice(args).await.unwrap_err(),
        "Invalid store_id UUID"
    );
}

#[tokio::test]
async fn rejects_a_store_with_no_enabled_payment_methods() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    // A store that is in scope but has no payment methods configured.
    let empty_store = StoreId(Uuid::new_v4());
    let server = h.server_scoped_to(vec![empty_store], StubRateProvider::usd_eth());

    assert_eq!(
        server
            .do_create_invoice(usd_args(empty_store))
            .await
            .unwrap_err(),
        "Store has no enabled payment methods"
    );
}

#[tokio::test]
async fn rejects_a_currency_no_payment_method_can_price() {
    // The provider knows USD/ETH but not JPY/ETH, so the ETH method is skipped
    // and nothing is left to price the invoice with.
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = usd_args(h.store_id);
    args.currency = "JPY".to_string();

    assert_eq!(
        h.server.do_create_invoice(args).await.unwrap_err(),
        "No payment methods with supported rate pairs for this currency"
    );
}

#[tokio::test]
async fn surfaces_rate_provider_failures_rather_than_skipping_the_method() {
    let h = TestHarness::new(StubRateProvider::unavailable());

    let err = h
        .server
        .do_create_invoice(usd_args(h.store_id))
        .await
        .unwrap_err();

    assert!(err.starts_with("Rate fetch failed"), "got: {err}");
}

#[tokio::test]
async fn rejects_a_non_positive_exchange_rate() {
    let h = TestHarness::new(StubRateProvider::new().with_rate("USD", "ETH", "0"));

    let err = h
        .server
        .do_create_invoice(usd_args(h.store_id))
        .await
        .unwrap_err();

    assert!(err.starts_with("Non-positive exchange rate"), "got: {err}");
}

#[tokio::test]
async fn rejects_a_non_positive_amount() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = usd_args(h.store_id);
    args.amount = "0".to_string();

    assert_eq!(
        h.server.do_create_invoice(args).await.unwrap_err(),
        "Amount must be positive"
    );
}

#[tokio::test]
async fn prices_a_crypto_denominated_invoice_without_a_rate() {
    let h = TestHarness::new(StubRateProvider::new());
    let mut args = usd_args(h.store_id);
    args.currency = "ETH".to_string();
    args.amount = "1.5".to_string();

    let json = parse_ok(h.server.do_create_invoice(args).await);

    let option = &json["payment_options"].as_array().unwrap()[0];
    assert_eq!(option["amount"], "1500000000000000000");
    assert!(option["rate"].is_null(), "same-asset needs no rate");
}

#[tokio::test]
async fn honours_a_custom_expiration() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = usd_args(h.store_id);
    args.expiration_seconds = Some(60);

    let json = parse_ok(h.server.do_create_invoice(args).await);

    let created: chrono::DateTime<chrono::Utc> =
        json["created_at"].as_str().unwrap().parse().unwrap();
    let expires: chrono::DateTime<chrono::Utc> =
        json["expires_at"].as_str().unwrap().parse().unwrap();
    let window = (expires - created).num_seconds();
    assert!((59..=61).contains(&window), "expiry window was {window}s");
}

#[tokio::test]
async fn defaults_to_the_standard_expiration() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let json = parse_ok(h.server.do_create_invoice(usd_args(h.store_id)).await);

    let created: chrono::DateTime<chrono::Utc> =
        json["created_at"].as_str().unwrap().parse().unwrap();
    let expires: chrono::DateTime<chrono::Utc> =
        json["expires_at"].as_str().unwrap().parse().unwrap();
    let expected = types::currency::DEFAULT_INVOICE_EXPIRATION_SECS as i64;
    let window = (expires - created).num_seconds();
    assert!(
        (expected - 1..=expected + 1).contains(&window),
        "expiry window was {window}s, expected ~{expected}s"
    );
}

#[tokio::test]
async fn folds_the_customer_email_into_metadata() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = usd_args(h.store_id);
    args.customer_email = Some("agent@example.com".to_string());
    args.metadata = Some(serde_json::json!({ "order_id": "A-1" }));

    let json = parse_ok(h.server.do_create_invoice(args).await);
    let invoice_id = types::InvoiceId::from_string(json["id"].as_str().unwrap().to_string());
    let stored = types::InvoiceReader::get(&*h.data, &invoice_id)
        .await
        .unwrap()
        .unwrap();

    let metadata = stored.metadata.unwrap();
    assert_eq!(metadata["order_id"], "A-1");
    assert_eq!(metadata["customer_email"], "agent@example.com");
}

#[tokio::test]
async fn keeps_a_caller_supplied_customer_email_in_metadata() {
    let h = TestHarness::new(StubRateProvider::usd_eth());
    let mut args = usd_args(h.store_id);
    args.customer_email = Some("outer@example.com".to_string());
    args.metadata = Some(serde_json::json!({ "customer_email": "inner@example.com" }));

    let json = parse_ok(h.server.do_create_invoice(args).await);
    let invoice_id = types::InvoiceId::from_string(json["id"].as_str().unwrap().to_string());
    let stored = types::InvoiceReader::get(&*h.data, &invoice_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        stored.metadata.unwrap()["customer_email"],
        "inner@example.com"
    );
}

#[tokio::test]
async fn advances_the_derivation_index_so_invoices_get_distinct_addresses() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let first = parse_ok(h.server.do_create_invoice(usd_args(h.store_id)).await);
    let second = parse_ok(h.server.do_create_invoice(usd_args(h.store_id)).await);

    let address_of = |json: &serde_json::Value| {
        json["payment_options"].as_array().unwrap()[0]["payment_address"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_ne!(address_of(&first), address_of(&second));
    assert_eq!(h.data.derivation_index(h.method_id), Some(2));
}

#[tokio::test]
async fn registers_the_payment_address_for_watching() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let json = parse_ok(h.server.do_create_invoice(usd_args(h.store_id)).await);
    let option = &json["payment_options"].as_array().unwrap()[0];
    let address = option["payment_address"].as_str().unwrap();

    let watched = types::WatchedAddressReader::get_invoice_id(
        &*h.data,
        address,
        crate::testkit::CHAIN_ID,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        watched.map(|id| id.0),
        Some(json["id"].as_str().unwrap().to_string())
    );
}
