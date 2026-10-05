//! In-memory MCP server harness.

use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;

use auth::UserId;
use data_service::InMemoryDataService;
use evm::monitor::bridge::MemoryBridge;
use types::{InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, StoreId};

use super::StubRateProvider;
use crate::server::{EthpayMcpServer, EvmMonitor, McpDataService};

/// BIP-32 account xpub used to derive payment addresses in tests.
///
/// Public key material only — the same vector the store API tests use.
pub const TEST_XPUB: &str = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";

/// Sepolia.
pub const CHAIN_ID: u64 = 11_155_111;

/// [`CHAIN_ID`] as the CAIP-2 identifier the payment method records.
pub fn caip2_chain_id() -> types::ChainId {
    types::ChainId::new("eip155", &CHAIN_ID.to_string()).expect("valid chain id")
}

/// A server wired to in-memory backends, plus handles for asserting on state.
pub struct TestHarness {
    pub server: EthpayMcpServer,
    pub data: Arc<InMemoryDataService>,
    /// The single store in the session scope.
    pub store_id: StoreId,
    /// The store's one enabled payment method (ETH, 18 decimals).
    pub method_id: Uuid,
}

impl TestHarness {
    /// One in-scope store with one enabled ETH payment method, no EVM monitor.
    pub fn new(rates: StubRateProvider) -> Self {
        Self::build(rates, vec![], None)
    }

    /// A server whose session scope is exactly `store_ids`, sharing this
    /// harness's data service. Used to test scopes the harness store isn't in.
    pub fn server_scoped_to(
        &self,
        store_ids: Vec<StoreId>,
        rates: StubRateProvider,
    ) -> EthpayMcpServer {
        EthpayMcpServer::new(
            Arc::clone(&self.data) as Arc<dyn McpDataService>,
            UserId(Uuid::new_v4()),
            store_ids,
            Arc::new(rates),
            None,
        )
    }

    /// A server serving exactly the session `validate_api_key` resolved: its
    /// user and store scope, with `monitor` attached. The harness's data
    /// service is shared, so the scope must name stores that exist in it.
    pub fn server_for_session(
        &self,
        user_id: UserId,
        store_ids: Vec<StoreId>,
        rates: StubRateProvider,
        monitor: Arc<MemoryBridge>,
    ) -> EthpayMcpServer {
        EthpayMcpServer::new(
            Arc::clone(&self.data) as Arc<dyn McpDataService>,
            user_id,
            store_ids,
            Arc::new(rates),
            Some(monitor as Arc<EvmMonitor>),
        )
    }

    fn build(
        rates: StubRateProvider,
        extra_store_ids: Vec<StoreId>,
        monitor: Option<Arc<MemoryBridge>>,
    ) -> Self {
        let data = Arc::new(InMemoryDataService::new());
        let store_id = StoreId(Uuid::new_v4());
        let method_id = data.add_payment_method(store_id.0, CHAIN_ID, "ETH", 18, TEST_XPUB);

        let mut store_ids = vec![store_id];
        store_ids.extend(extra_store_ids);

        let server = EthpayMcpServer::new(
            Arc::clone(&data) as Arc<dyn McpDataService>,
            UserId(Uuid::new_v4()),
            store_ids,
            Arc::new(rates),
            monitor.map(|m| m as Arc<EvmMonitor>),
        );

        Self {
            server,
            data,
            store_id,
            method_id,
        }
    }

    /// A store ID that is in no session scope.
    pub fn foreign_store_id() -> StoreId {
        StoreId(Uuid::new_v4())
    }

    /// Seed an invoice directly into the data service, bypassing the tools.
    pub async fn seed_invoice(
        &self,
        store_id: StoreId,
        currency: &str,
        amount: &str,
        status: InvoiceStatus,
    ) -> InvoiceId {
        let invoice = InvoiceData {
            id: InvoiceId::new(),
            store_id,
            currency: currency.to_string(),
            status,
            amount: amount.to_string(),
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::seconds(900),
            metadata: None,
            customer_email: None,
            extra: None,
        };
        InvoiceWriter::upsert(&*self.data, &invoice).await.unwrap();
        invoice.id
    }
}

/// Parse a handler's JSON payload, failing loudly on an error response.
pub fn parse_ok(result: Result<String, String>) -> serde_json::Value {
    let json = result.expect("handler returned an error");
    serde_json::from_str(&json).expect("handler returned invalid JSON")
}
