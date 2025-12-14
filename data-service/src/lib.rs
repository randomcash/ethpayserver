//! Data access layer for PayServer.
//!
//! This crate provides database abstractions for storing and retrieving
//! invoices, payments, and related data.

use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use types::{Network, InvoiceData, InvoiceId, InvoiceStatus, PaymentData};
use thiserror::Error;
use uuid::Uuid;

pub mod postgres;

/// Data service error types.
#[derive(Debug, Error)]
pub enum DataServiceError {
    #[error("database error: {0}")]
    Database(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<sqlx::Error> for DataServiceError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => DataServiceError::NotFound("row not found".into()),
            _ => DataServiceError::Database(e.to_string()),
        }
    }
}

/// Result type for data service operations.
pub type DataServiceResult<T> = Result<T, DataServiceError>;

/// Trait for accessing a data service from a context.
pub trait DataServiceAccess<DS> {
    fn data_service(&self) -> &DS;
}

impl<DS, T> DataServiceAccess<DS> for Arc<T>
where
    T: DataServiceAccess<DS>,
{
    fn data_service(&self) -> &DS {
        self.deref().data_service()
    }
}

impl<DS, T> DataServiceAccess<DS> for Pin<Box<T>>
where
    T: DataServiceAccess<DS>,
{
    fn data_service(&self) -> &DS {
        self.deref().data_service()
    }
}

/// Query parameters for listing invoices.
#[derive(Debug, Clone, Default)]
pub struct InvoiceQueryParams {
    pub status: Option<InvoiceStatus>,
    pub network: Option<Network>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

impl InvoiceQueryParams {
    pub fn new() -> Self {
        Self {
            status: None,
            network: None,
            created_after: None,
            created_before: None,
            limit: 50,
            offset: 0,
        }
    }

    pub fn with_status(mut self, status: InvoiceStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_offset(mut self, offset: i64) -> Self {
        self.offset = offset;
        self
    }
}

/// Query parameters for listing payments.
#[derive(Debug, Clone, Default)]
pub struct PaymentQueryParams {
    pub invoice_id: Option<InvoiceId>,
    pub min_confirmations: Option<u32>,
    pub limit: i64,
    pub offset: i64,
}

/// Core data service trait for payment server persistence.
pub trait DataService: Send + Sync {
    type Error: std::fmt::Debug + ToString + Send;

    /// Insert a new invoice.
    fn insert_invoice<'a>(
        &'a self,
        invoice: &'a InvoiceData,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get an invoice by ID.
    fn get_invoice<'a>(
        &'a self,
        id: &'a InvoiceId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<InvoiceData>, Self::Error>> + Send + 'a>>;

    /// Update an invoice's status.
    fn update_invoice_status<'a>(
        &'a self,
        id: &'a InvoiceId,
        status: InvoiceStatus,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Update the amount received for an invoice.
    fn update_invoice_amount_received<'a>(
        &'a self,
        id: &'a InvoiceId,
        amount_received: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Query invoices with pagination.
    fn query_invoices<'a>(
        &'a self,
        params: &'a InvoiceQueryParams,
    ) -> Pin<Box<dyn Future<Output = Result<(i64, Vec<InvoiceData>), Self::Error>> + Send + 'a>>;

    /// Get all pending invoices that have expired.
    fn get_expired_invoices<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<InvoiceData>, Self::Error>> + Send + 'a>>;

    /// Insert a new payment.
    fn insert_payment<'a>(
        &'a self,
        payment: &'a PaymentData,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get a payment by ID.
    fn get_payment<'a>(
        &'a self,
        id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<Option<PaymentData>, Self::Error>> + Send + 'a>>;

    /// Get all payments for an invoice.
    fn get_payments_for_invoice<'a>(
        &'a self,
        invoice_id: &'a InvoiceId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PaymentData>, Self::Error>> + Send + 'a>>;

    /// Update payment confirmations.
    fn update_payment_confirmations<'a>(
        &'a self,
        id: Uuid,
        confirmations: u32,
        confirmed_at: Option<DateTime<Utc>>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get payments with fewer than N confirmations (for monitoring).
    fn get_unconfirmed_payments<'a>(
        &'a self,
        min_confirmations: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PaymentData>, Self::Error>> + Send + 'a>>;

    /// Store a watched address.
    fn insert_watched_address<'a>(
        &'a self,
        address: &'a str,
        invoice_id: &'a InvoiceId,
        network: Network,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get the invoice ID associated with an address.
    fn get_invoice_by_address<'a>(
        &'a self,
        address: &'a str,
        network: Network,
    ) -> Pin<Box<dyn Future<Output = Result<Option<InvoiceId>, Self::Error>> + Send + 'a>>;

    /// Get all active watched addresses.
    fn get_active_watched_addresses<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<(String, InvoiceId, Network)>, Self::Error>> + Send + 'a>>;

    /// Remove a watched address.
    fn remove_watched_address<'a>(
        &'a self,
        address: &'a str,
        network: Network,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;
}

/// Test utilities for data service.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils {
    use super::*;
    use std::collections::HashMap;
    use std::sync::RwLock;

    /// In-memory implementation of DataService for testing.
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

    impl DataService for InMemoryDataService {
        type Error = DataServiceError;

        fn insert_invoice<'a>(
            &'a self,
            invoice: &'a InvoiceData,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut invoices = self.invoices.write().unwrap();
                invoices.insert(invoice.id.0.clone(), invoice.clone());
                Ok(())
            })
        }

        fn get_invoice<'a>(
            &'a self,
            id: &'a InvoiceId,
        ) -> Pin<Box<dyn Future<Output = Result<Option<InvoiceData>, Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
                let invoices = self.invoices.read().unwrap();
                Ok(invoices.get(&id.0).cloned())
            })
        }

        fn update_invoice_status<'a>(
            &'a self,
            id: &'a InvoiceId,
            status: InvoiceStatus,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut invoices = self.invoices.write().unwrap();
                if let Some(invoice) = invoices.get_mut(&id.0) {
                    invoice.status = status;
                }
                Ok(())
            })
        }

        fn update_invoice_amount_received<'a>(
            &'a self,
            id: &'a InvoiceId,
            amount_received: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut invoices = self.invoices.write().unwrap();
                if let Some(invoice) = invoices.get_mut(&id.0) {
                    invoice.amount_received = amount_received.to_string();
                }
                Ok(())
            })
        }

        fn query_invoices<'a>(
            &'a self,
            params: &'a InvoiceQueryParams,
        ) -> Pin<Box<dyn Future<Output = Result<(i64, Vec<InvoiceData>), Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
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
            })
        }

        fn get_expired_invoices<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<InvoiceData>, Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let invoices = self.invoices.read().unwrap();
                let now = Utc::now();
                Ok(invoices
                    .values()
                    .filter(|inv| inv.status == InvoiceStatus::Pending && inv.expires_at < now)
                    .cloned()
                    .collect())
            })
        }

        fn insert_payment<'a>(
            &'a self,
            payment: &'a PaymentData,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut payments = self.payments.write().unwrap();
                payments.insert(payment.id, payment.clone());
                Ok(())
            })
        }

        fn get_payment<'a>(
            &'a self,
            id: Uuid,
        ) -> Pin<Box<dyn Future<Output = Result<Option<PaymentData>, Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
                let payments = self.payments.read().unwrap();
                Ok(payments.get(&id).cloned())
            })
        }

        fn get_payments_for_invoice<'a>(
            &'a self,
            invoice_id: &'a InvoiceId,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<PaymentData>, Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let payments = self.payments.read().unwrap();
                Ok(payments
                    .values()
                    .filter(|p| p.invoice_id == *invoice_id)
                    .cloned()
                    .collect())
            })
        }

        fn update_payment_confirmations<'a>(
            &'a self,
            id: Uuid,
            confirmations: u32,
            confirmed_at: Option<DateTime<Utc>>,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut payments = self.payments.write().unwrap();
                if let Some(payment) = payments.get_mut(&id) {
                    payment.confirmations = confirmations;
                    if confirmed_at.is_some() {
                        payment.confirmed_at = confirmed_at;
                    }
                }
                Ok(())
            })
        }

        fn get_unconfirmed_payments<'a>(
            &'a self,
            min_confirmations: u32,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<PaymentData>, Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let payments = self.payments.read().unwrap();
                Ok(payments
                    .values()
                    .filter(|p| p.confirmations < min_confirmations)
                    .cloned()
                    .collect())
            })
        }

        fn insert_watched_address<'a>(
            &'a self,
            address: &'a str,
            invoice_id: &'a InvoiceId,
            network: Network,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut addresses = self.addresses.write().unwrap();
                addresses.insert((address.to_string(), network), invoice_id.clone());
                Ok(())
            })
        }

        fn get_invoice_by_address<'a>(
            &'a self,
            address: &'a str,
            network: Network,
        ) -> Pin<Box<dyn Future<Output = Result<Option<InvoiceId>, Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
                let addresses = self.addresses.read().unwrap();
                Ok(addresses.get(&(address.to_string(), network)).cloned())
            })
        }

        fn get_active_watched_addresses<'a>(
            &'a self,
        ) -> Pin<
            Box<dyn Future<Output = Result<Vec<(String, InvoiceId, Network)>, Self::Error>> + Send + 'a>,
        > {
            Box::pin(async move {
                let addresses = self.addresses.read().unwrap();
                Ok(addresses
                    .iter()
                    .map(|((addr, network), id)| (addr.clone(), id.clone(), *network))
                    .collect())
            })
        }

        fn remove_watched_address<'a>(
            &'a self,
            address: &'a str,
            network: Network,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut addresses = self.addresses.write().unwrap();
                addresses.remove(&(address.to_string(), network));
                Ok(())
            })
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
}

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use super::*;

    #[tokio::test]
    async fn test_in_memory_data_service() {
        let ds = InMemoryDataService::new();
        let invoice = create_test_invoice();

        // Insert
        ds.insert_invoice(&invoice).await.unwrap();

        // Get
        let retrieved = ds.get_invoice(&invoice.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, invoice.id);

        // Update status
        ds.update_invoice_status(&invoice.id, InvoiceStatus::Paid)
            .await
            .unwrap();
        let updated = ds.get_invoice(&invoice.id).await.unwrap().unwrap();
        assert_eq!(updated.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn test_watched_addresses() {
        let ds = InMemoryDataService::new();
        let invoice_id = InvoiceId::new();
        let address = "0x1234567890abcdef1234567890abcdef12345678";
        let network = Network::Ethereum;

        ds.insert_watched_address(address, &invoice_id, network)
            .await
            .unwrap();

        let found = ds.get_invoice_by_address(address, network).await.unwrap();
        assert_eq!(found, Some(invoice_id.clone()));

        ds.remove_watched_address(address, network).await.unwrap();
        let found = ds.get_invoice_by_address(address, network).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_query_by_network() {
        let ds = InMemoryDataService::new();

        let eth_invoice = create_test_invoice();
        ds.insert_invoice(&eth_invoice).await.unwrap();

        let mut btc_invoice = create_test_invoice();
        btc_invoice.id = InvoiceId::new();
        btc_invoice.network = Network::BitcoinMainnet;
        btc_invoice.asset_symbol = "BTC".to_string();
        ds.insert_invoice(&btc_invoice).await.unwrap();

        // Query all
        let params = InvoiceQueryParams::new();
        let (total, _) = ds.query_invoices(&params).await.unwrap();
        assert_eq!(total, 2);

        // Query ETH only
        let params = InvoiceQueryParams::new().with_network(Network::Ethereum);
        let (total, invoices) = ds.query_invoices(&params).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(invoices[0].network, Network::Ethereum);
    }
}
