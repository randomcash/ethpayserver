//! Test utilities for data service.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use types::{
    InvoiceData, InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter,
    Network, PaymentData, PaymentReader, PaymentWriter, RepositoryResult, WatchedAddressReader,
    WatchedAddressWriter,
};
use uuid::Uuid;

/// In-memory implementation of all repository traits for testing.
#[derive(Default)]
pub struct InMemoryDataService {
    invoices: RwLock<HashMap<String, InvoiceData>>,
    payments: RwLock<HashMap<Uuid, PaymentData>>,
    addresses: RwLock<HashMap<(String, Network), InvoiceId>>,
}

impl InMemoryDataService {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl InvoiceReader for InMemoryDataService {
    async fn get(&self, id: &InvoiceId) -> RepositoryResult<Option<InvoiceData>> {
        let invoices = self.invoices.read().unwrap();
        Ok(invoices.get(&id.0).cloned())
    }

    async fn query(
        &self,
        params: &InvoiceQueryParams,
    ) -> RepositoryResult<(i64, Vec<InvoiceData>)> {
        let invoices = self.invoices.read().unwrap();
        let mut results: Vec<InvoiceData> = invoices
            .values()
            .filter(|inv| {
                if let Some(status) = params.status {
                    if inv.status != status {
                        return false;
                    }
                }
                if let Some(network) = params.network {
                    if inv.network != network {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        let total = results.len() as i64;
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let offset = params.offset as usize;
        let limit = params.limit as usize;
        let results = results.into_iter().skip(offset).take(limit).collect();

        Ok((total, results))
    }

    async fn get_expired(&self) -> RepositoryResult<Vec<InvoiceData>> {
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();
        Ok(invoices
            .values()
            .filter(|inv| inv.status == InvoiceStatus::Pending && inv.expires_at < now)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl InvoiceWriter for InMemoryDataService {
    async fn upsert(&self, invoice: &InvoiceData) -> RepositoryResult<()> {
        let mut invoices = self.invoices.write().unwrap();
        invoices.insert(invoice.id.0.clone(), invoice.clone());
        Ok(())
    }

    async fn update_status(&self, id: &InvoiceId, status: InvoiceStatus) -> RepositoryResult<()> {
        let mut invoices = self.invoices.write().unwrap();
        if let Some(invoice) = invoices.get_mut(&id.0) {
            invoice.status = status;
        }
        Ok(())
    }

    async fn update_amount_received(&self, id: &InvoiceId, amount: &str) -> RepositoryResult<()> {
        let mut invoices = self.invoices.write().unwrap();
        if let Some(invoice) = invoices.get_mut(&id.0) {
            invoice.amount_received = amount.to_string();
        }
        Ok(())
    }
}

#[async_trait]
impl PaymentReader for InMemoryDataService {
    async fn get(&self, id: Uuid) -> RepositoryResult<Option<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments.get(&id).cloned())
    }

    async fn get_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments
            .values()
            .filter(|p| p.invoice_id == *invoice_id)
            .cloned()
            .collect())
    }

    async fn get_unconfirmed(&self, min_confirmations: u32) -> RepositoryResult<Vec<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments
            .values()
            .filter(|p| p.confirmations < min_confirmations)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl PaymentWriter for InMemoryDataService {
    async fn upsert(&self, payment: &PaymentData) -> RepositoryResult<()> {
        let mut payments = self.payments.write().unwrap();
        payments.insert(payment.id, payment.clone());
        Ok(())
    }

    async fn update_confirmations(
        &self,
        id: Uuid,
        confirmations: u32,
        confirmed_at: Option<DateTime<Utc>>,
    ) -> RepositoryResult<()> {
        let mut payments = self.payments.write().unwrap();
        if let Some(payment) = payments.get_mut(&id) {
            payment.confirmations = confirmations;
            if confirmed_at.is_some() {
                payment.confirmed_at = confirmed_at;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl WatchedAddressReader for InMemoryDataService {
    async fn get_invoice_id(
        &self,
        address: &str,
        network: Network,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses.get(&(address.to_string(), network)).cloned())
    }

    async fn get_active(&self) -> RepositoryResult<Vec<(String, InvoiceId, Network)>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses
            .iter()
            .map(|((addr, network), id)| (addr.clone(), id.clone(), *network))
            .collect())
    }
}

#[async_trait]
impl WatchedAddressWriter for InMemoryDataService {
    async fn upsert(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
    ) -> RepositoryResult<()> {
        let mut addresses = self.addresses.write().unwrap();
        addresses.insert((address.to_string(), network), invoice_id.clone());
        Ok(())
    }

    async fn remove(&self, address: &str, network: Network) -> RepositoryResult<()> {
        let mut addresses = self.addresses.write().unwrap();
        addresses.remove(&(address.to_string(), network));
        Ok(())
    }
}

/// Create a test invoice.
pub fn create_test_invoice() -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        network: Network::Ethereum,
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(), // 1 ETH in wei
        amount_received: "0".to_string(),
        asset_symbol: "ETH".to_string(),
        payment_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        payment_request: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    }
}

/// Create a test payment.
pub fn create_test_payment(invoice_id: &InvoiceId) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        network: Network::Ethereum,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        tx_hash: "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
        block_number: Some(12345678),
        confirmations: 0,
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
        extra: None,
    }
}
