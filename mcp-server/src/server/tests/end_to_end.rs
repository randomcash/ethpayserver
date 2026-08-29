//! End-to-end agent flow over the MCP tool surface.
//!
//! Drives the same path a real agent takes: authenticate an API key, then call
//! the `#[tool]` entry points (not the `do_*` helpers) and read back the JSON
//! they hand the agent.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;

use evm::monitor::bridge::MemoryBridge;
use evm::monitor::events::MonitorCommand;

use crate::api_key::validate_api_key;
use crate::server::{
    CancelInvoiceArgs, CreateInvoiceArgs, GetInvoiceArgs, GetPaymentStatusArgs, ListInvoicesArgs,
};
use crate::testkit::{
    RAW_KEY, StubAuthRepo, StubRateProvider, TestHarness, test_api_key, test_store,
};

fn json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("tool returned invalid JSON")
}

#[tokio::test]
async fn agent_authenticates_then_creates_and_settles_an_invoice() {
    // --- Authenticate, exactly as `main` does at start-up -------------------
    let user_id = auth::UserId(uuid::Uuid::new_v4());
    let store = test_store(user_id);
    let auth_repo = StubAuthRepo::with_key(test_api_key(user_id)).with_store(store);
    let (_, store_ids) = validate_api_key(&auth_repo, RAW_KEY)
        .await
        .expect("API key should validate");
    assert_eq!(store_ids.len(), 1, "the key's scope is one store");

    // --- Serve tools with an EVM monitor attached ---------------------------
    let bridge = Arc::new(MemoryBridge::new());
    // Subscribe before any command is published — the bridge is a broadcast.
    let mut commands = bridge.commands_sender().subscribe();
    let h = TestHarness::with_monitor(StubRateProvider::usd_eth(), Arc::clone(&bridge));

    // --- create_invoice -----------------------------------------------------
    let created = json(
        &h.server
            .create_invoice(Parameters(CreateInvoiceArgs {
                store_id: h.store_id.0.to_string(),
                currency: "USD".to_string(),
                amount: "25.00".to_string(),
                expiration_seconds: Some(600),
                metadata: Some(serde_json::json!({ "agent": "test-agent" })),
                customer_email: Some("agent@example.com".to_string()),
            }))
            .await,
    );
    let invoice_id = created["id"].as_str().unwrap().to_string();
    let option = &created["payment_options"].as_array().unwrap()[0];
    let payment_address = option["payment_address"].as_str().unwrap().to_string();

    assert_eq!(created["status"], "pending");
    // 25.00 USD * 0.0005 ETH/USD = 0.0125 ETH
    assert_eq!(option["amount"], "12500000000000000");

    // --- the monitor was told to watch the derived address ------------------
    let command = commands
        .try_recv()
        .expect("a WatchAddress command should have been published");
    match command {
        MonitorCommand::WatchAddress(watch) => {
            assert_eq!(watch.address.to_string(), payment_address);
            assert_eq!(watch.invoice_id.to_string(), invoice_id);
            assert_eq!(watch.chain_id, crate::testkit::CHAIN_ID);
            assert_eq!(
                watch.expected_amount,
                Some("12500000000000000".parse::<evm::U256>().unwrap())
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    // --- list_invoices sees it ----------------------------------------------
    let listed = json(
        &h.server
            .list_invoices(Parameters(ListInvoicesArgs {
                store_id: h.store_id.0.to_string(),
                status: Some("pending".to_string()),
                currency: Some("USD".to_string()),
                limit: None,
                offset: None,
            }))
            .await,
    );
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["invoices"].as_array().unwrap()[0]["id"], invoice_id);

    // --- get_invoice round-trips the payment option -------------------------
    let fetched = json(
        &h.server
            .get_invoice(Parameters(GetInvoiceArgs {
                invoice_id: invoice_id.clone(),
            }))
            .await,
    );
    assert_eq!(fetched["store_id"], h.store_id.0.to_string());
    assert_eq!(fetched["metadata"]["agent"], "test-agent");
    assert_eq!(fetched["metadata"]["customer_email"], "agent@example.com");
    assert_eq!(
        fetched["payment_options"].as_array().unwrap()[0]["payment_address"],
        payment_address
    );

    // --- get_payment_status is what an agent polls --------------------------
    let status = json(
        &h.server
            .get_payment_status(Parameters(GetPaymentStatusArgs {
                invoice_id: invoice_id.clone(),
            }))
            .await,
    );
    assert_eq!(status["status"], "pending");
    assert_eq!(status["amount"], "25.00");
    assert_eq!(status["payment_count"], 0);
    assert_eq!(status["is_paid"], false);
    assert_eq!(status["is_expired"], false);

    // --- cancel_invoice ends the flow ---------------------------------------
    let cancelled = json(
        &h.server
            .cancel_invoice(Parameters(CancelInvoiceArgs {
                invoice_id: invoice_id.clone(),
            }))
            .await,
    );
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["previous_status"], "pending");

    let status = json(
        &h.server
            .get_payment_status(Parameters(GetPaymentStatusArgs { invoice_id }))
            .await,
    );
    assert_eq!(status["status"], "cancelled");
}

#[tokio::test]
async fn tool_errors_reach_the_agent_as_json() {
    let h = TestHarness::new(StubRateProvider::usd_eth());

    let response = json(
        &h.server
            .create_invoice(Parameters(CreateInvoiceArgs {
                store_id: TestHarness::foreign_store_id().0.to_string(),
                currency: "USD".to_string(),
                amount: "10.00".to_string(),
                expiration_seconds: None,
                metadata: None,
                customer_email: None,
            }))
            .await,
    );

    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .starts_with("Unauthorized"),
        "got: {response}"
    );
}

#[tokio::test]
async fn a_key_with_no_stores_cannot_reach_any_invoice() {
    // A key whose owner owns no store resolves to an empty scope, so every
    // tool call is refused.
    let user_id = auth::UserId(uuid::Uuid::new_v4());
    let auth_repo = StubAuthRepo::with_key(test_api_key(user_id));
    let (_, store_ids) = validate_api_key(&auth_repo, RAW_KEY).await.unwrap();
    assert!(store_ids.is_empty());

    let h = TestHarness::new(StubRateProvider::usd_eth());
    let invoice_id = h
        .seed_invoice(h.store_id, "USD", "10.00", types::InvoiceStatus::Pending)
        .await;
    let server = h.server_scoped_to(store_ids, StubRateProvider::usd_eth());

    let response = json(
        &server
            .get_invoice(Parameters(GetInvoiceArgs {
                invoice_id: invoice_id.0,
            }))
            .await,
    );

    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .starts_with("Unauthorized"),
        "got: {response}"
    );
}
