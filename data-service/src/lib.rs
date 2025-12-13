//! Data access layer for PayServer.
//!
//! This crate provides database abstractions for storing and retrieving
//! invoices, payments, and related data.

use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::{Invoice, InvoiceId, InvoiceStatus, Payment, PaymentMethod};
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
    pub payment_method: Option<PaymentMethod>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

impl InvoiceQueryParams {
    pub fn new() -> Self {
        Self {
            status: None,
            payment_method: None,
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
        invoice: &'a Invoice,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get an invoice by ID.
    fn get_invoice<'a>(
        &'a self,
        id: &'a InvoiceId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Invoice>, Self::Error>> + Send + 'a>>;

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
        amount_received: i64,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Query invoices with pagination.
    fn query_invoices<'a>(
        &'a self,
        params: &'a InvoiceQueryParams,
    ) -> Pin<Box<dyn Future<Output = Result<(i64, Vec<Invoice>), Self::Error>> + Send + 'a>>;

    /// Get all pending invoices that have expired.
    fn get_expired_invoices<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Invoice>, Self::Error>> + Send + 'a>>;

    /// Insert a new payment.
    fn insert_payment<'a>(
        &'a self,
        payment: &'a Payment,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get a payment by ID.
    fn get_payment<'a>(
        &'a self,
        id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Payment>, Self::Error>> + Send + 'a>>;

    /// Get all payments for an invoice.
    fn get_payments_for_invoice<'a>(
        &'a self,
        invoice_id: &'a InvoiceId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Payment>, Self::Error>> + Send + 'a>>;

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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Payment>, Self::Error>> + Send + 'a>>;

    /// Store a watched address.
    fn insert_watched_address<'a>(
        &'a self,
        address: &'a str,
        invoice_id: &'a InvoiceId,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;

    /// Get the invoice ID associated with an address.
    fn get_invoice_by_address<'a>(
        &'a self,
        address: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<InvoiceId>, Self::Error>> + Send + 'a>>;

    /// Get all active watched addresses.
    fn get_active_watched_addresses<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<(String, InvoiceId)>, Self::Error>> + Send + 'a>>;

    /// Remove a watched address.
    fn remove_watched_address<'a>(
        &'a self,
        address: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>>;
}

/// Test utilities for data service.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils {
    use super::*;
    use common::Amount;
    use std::collections::HashMap;
    use std::sync::RwLock;

    /// In-memory implementation of DataService for testing.
    #[derive(Default)]
    pub struct InMemoryDataService {
        invoices: RwLock<HashMap<String, Invoice>>,
        payments: RwLock<HashMap<Uuid, Payment>>,
        addresses: RwLock<HashMap<String, InvoiceId>>,
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
            invoice: &'a Invoice,
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
        ) -> Pin<Box<dyn Future<Output = Result<Option<Invoice>, Self::Error>> + Send + 'a>>
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
            amount_received: i64,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut invoices = self.invoices.write().unwrap();
                if let Some(invoice) = invoices.get_mut(&id.0) {
                    invoice.amount_received = amount_received;
                }
                Ok(())
            })
        }

        fn query_invoices<'a>(
            &'a self,
            params: &'a InvoiceQueryParams,
        ) -> Pin<Box<dyn Future<Output = Result<(i64, Vec<Invoice>), Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
                let invoices = self.invoices.read().unwrap();
                let mut results: Vec<Invoice> = invoices
                    .values()
                    .filter(|inv| {
                        if let Some(status) = params.status {
                            if inv.status != status {
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
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Invoice>, Self::Error>> + Send + 'a>> {
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
            payment: &'a Payment,
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
        ) -> Pin<Box<dyn Future<Output = Result<Option<Payment>, Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
                let payments = self.payments.read().unwrap();
                Ok(payments.get(&id).cloned())
            })
        }

        fn get_payments_for_invoice<'a>(
            &'a self,
            invoice_id: &'a InvoiceId,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Payment>, Self::Error>> + Send + 'a>> {
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
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Payment>, Self::Error>> + Send + 'a>> {
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
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut addresses = self.addresses.write().unwrap();
                addresses.insert(address.to_string(), invoice_id.clone());
                Ok(())
            })
        }

        fn get_invoice_by_address<'a>(
            &'a self,
            address: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Option<InvoiceId>, Self::Error>> + Send + 'a>>
        {
            Box::pin(async move {
                let addresses = self.addresses.read().unwrap();
                Ok(addresses.get(address).cloned())
            })
        }

        fn get_active_watched_addresses<'a>(
            &'a self,
        ) -> Pin<
            Box<dyn Future<Output = Result<Vec<(String, InvoiceId)>, Self::Error>> + Send + 'a>,
        > {
            Box::pin(async move {
                let addresses = self.addresses.read().unwrap();
                Ok(addresses
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect())
            })
        }

        fn remove_watched_address<'a>(
            &'a self,
            address: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'a>> {
            Box::pin(async move {
                let mut addresses = self.addresses.write().unwrap();
                addresses.remove(address);
                Ok(())
            })
        }
    }

    /// Create a test invoice.
    pub fn create_test_invoice() -> Invoice {
        Invoice {
            id: InvoiceId::new(),
            amount: Amount::btc(100_000),
            payment_methods: vec![common::PaymentMethod::BitcoinOnChain],
            status: InvoiceStatus::Pending,
            payment_address: Some("bc1qtest...".to_string()),
            payment_request: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            amount_received: 0,
            metadata: None,
            webhook_url: None,
            redirect_url: None,
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
        let address = "bc1qtest123";

        ds.insert_watched_address(address, &invoice_id)
            .await
            .unwrap();

        let found = ds.get_invoice_by_address(address).await.unwrap();
        assert_eq!(found, Some(invoice_id.clone()));

        ds.remove_watched_address(address).await.unwrap();
        let found = ds.get_invoice_by_address(address).await.unwrap();
        assert!(found.is_none());
    }
}
