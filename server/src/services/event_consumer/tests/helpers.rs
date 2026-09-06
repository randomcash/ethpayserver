#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_trait::async_trait;
use chrono::Utc;
use data_service::InMemoryDataService;
use evm::{Address, U256};
use std::sync::Arc;
use types::{InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, Network, StoreId};

use crate::services::email;
use crate::services::evm_monitor::{EVMMonitor, EVMMonitorError};

/// Get the native asset symbol for a network (test helper).
pub fn network_native_symbol(network: types::Network) -> String {
    use types::Network::*;
    match network {
        Ethereum | Arbitrum | Optimism | Base | ZkSync | Linea | Scroll => "ETH",
        Polygon => "POL",
        Avalanche => "AVAX",
        BinanceSmartChain => "BNB",
        Fantom => "FTM",
        Gnosis => "xDAI",
        // Non-EVM networks - shouldn't reach here
        _ => "UNKNOWN",
    }
    .to_string()
}

/// Mock EVMMonitor for testing.
pub struct MockEVMMonitor;

#[async_trait]
impl EVMMonitor for MockEVMMonitor {
    async fn watch_address(
        &self,
        _network: Network,
        _address: Address,
        _invoice_id: uuid::Uuid,
        _expected_amount: Option<U256>,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn watch_address_by_chain_id(
        &self,
        _chain_id: u64,
        _address: Address,
        _invoice_id: uuid::Uuid,
        _expected_amount: Option<U256>,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn unwatch_address(
        &self,
        _network: Network,
        _address: Address,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn unwatch_address_by_chain_id(
        &self,
        _chain_id: u64,
        _address: Address,
        _token_contract: Option<Address>,
    ) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn health_check(&self) -> Result<(), EVMMonitorError> {
        Ok(())
    }

    async fn get_chain_health(&self) -> Result<Vec<evm::monitor::ChainHealth>, EVMMonitorError> {
        Ok(vec![])
    }
}

/// Create a test invoice in the data service.
pub async fn create_test_invoice(
    ds: &InMemoryDataService,
    invoice_id: &InvoiceId,
    store_id: StoreId,
) {
    let invoice = InvoiceData {
        id: invoice_id.clone(),
        store_id,
        currency: "ETH".to_string(),
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(), // 1 ETH
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        customer_email: None,
        extra: None,
    };
    InvoiceWriter::upsert(ds, &invoice).await.unwrap();
}

/// Mock email sender that records calls for test assertions.
pub struct MockEmailSender {
    calls: std::sync::Mutex<Vec<(String, String)>>, // (to, invoice_id)
}

impl MockEmailSender {
    pub fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn calls(&self) -> std::sync::MutexGuard<'_, Vec<(String, String)>> {
        self.calls.lock().unwrap()
    }
}

#[async_trait]
impl email::EmailSender for MockEmailSender {
    async fn send_receipt(
        &self,
        to: &str,
        data: &email::ReceiptData,
    ) -> Result<(), email::EmailError> {
        self.calls
            .lock()
            .unwrap()
            .push((to.to_string(), data.invoice_id.clone()));
        Ok(())
    }
}

/// Create a consumer with in-memory data service and no-op email.
pub fn create_test_consumer(
    ds: Arc<InMemoryDataService>,
    bridge: Arc<evm::monitor::bridge::MemoryBridge>,
) -> super::super::EventConsumer<InMemoryDataService, MockEVMMonitor> {
    super::super::EventConsumer::new(
        bridge,
        ds,
        None,
        None,
        None,
        Arc::new(email::NoopEmailSender),
    )
}
