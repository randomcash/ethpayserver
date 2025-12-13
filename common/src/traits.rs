//! Core traits for the PayServer ecosystem.

use std::future::Future;
use std::pin::Pin;

use crate::error::PayServerResult;
use crate::types::{
    Currency, HealthStatus, Invoice, InvoiceId, InvoiceStatus, Payment, PaymentEvent,
    PaymentMethod,
};

/// Configuration for creating an invoice.
#[derive(Debug, Clone)]
pub struct CreateInvoiceRequest {
    /// Amount to request (in smallest currency unit).
    pub amount: i64,
    /// Currency for the invoice.
    pub currency: Currency,
    /// Accepted payment methods (if empty, all supported methods are accepted).
    pub payment_methods: Vec<PaymentMethod>,
    /// Invoice expiration in seconds from now.
    pub expiration_seconds: Option<u64>,
    /// Optional metadata to attach to the invoice.
    pub metadata: Option<serde_json::Value>,
    /// Optional webhook URL for payment notifications.
    pub webhook_url: Option<String>,
    /// Optional redirect URL after successful payment.
    pub redirect_url: Option<String>,
}

impl CreateInvoiceRequest {
    pub fn new(amount: i64, currency: Currency) -> Self {
        Self {
            amount,
            currency,
            payment_methods: vec![],
            expiration_seconds: None,
            metadata: None,
            webhook_url: None,
            redirect_url: None,
        }
    }

    pub fn with_expiration(mut self, seconds: u64) -> Self {
        self.expiration_seconds = Some(seconds);
        self
    }

    pub fn with_payment_methods(mut self, methods: Vec<PaymentMethod>) -> Self {
        self.payment_methods = methods;
        self
    }

    pub fn with_webhook(mut self, url: String) -> Self {
        self.webhook_url = Some(url);
        self
    }

    pub fn with_redirect(mut self, url: String) -> Self {
        self.redirect_url = Some(url);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Query parameters for listing invoices.
#[derive(Debug, Clone, Default)]
pub struct InvoiceQuery {
    /// Filter by status.
    pub status: Option<InvoiceStatus>,
    /// Filter by currency.
    pub currency: Option<Currency>,
    /// Maximum number of results.
    pub limit: Option<u32>,
    /// Offset for pagination.
    pub offset: Option<u32>,
}

/// Core trait that all payment servers must implement.
pub trait PayServer: Send + Sync {
    /// Create a new invoice.
    fn create_invoice(
        &self,
        request: CreateInvoiceRequest,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<Invoice>> + Send + '_>>;

    /// Get an invoice by ID.
    fn get_invoice(
        &self,
        id: &InvoiceId,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<Invoice>> + Send + '_>>;

    /// Cancel an invoice.
    fn cancel_invoice(
        &self,
        id: &InvoiceId,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;

    /// List invoices matching the query.
    fn list_invoices(
        &self,
        query: InvoiceQuery,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<Vec<Invoice>>> + Send + '_>>;

    /// Get all payments for an invoice.
    fn get_payments(
        &self,
        invoice_id: &InvoiceId,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<Vec<Payment>>> + Send + '_>>;

    /// Get health status of the payment server.
    fn health(&self) -> Pin<Box<dyn Future<Output = PayServerResult<HealthStatus>> + Send + '_>>;

    /// Get supported currencies.
    fn supported_currencies(&self) -> Vec<Currency>;

    /// Get supported payment methods.
    fn supported_payment_methods(&self) -> Vec<PaymentMethod>;
}

/// Trait for monitoring blockchain for payments.
pub trait PaymentMonitor: Send + Sync {
    /// Start monitoring for payments.
    fn start(&self) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;

    /// Stop monitoring.
    fn stop(&self) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;

    /// Check if the monitor is running.
    fn is_running(&self) -> bool;

    /// Get the current block height being monitored.
    fn current_block_height(
        &self,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<u64>> + Send + '_>>;
}

/// Trait for subscribing to payment events.
pub trait PaymentEventSubscriber: Send + Sync {
    /// Subscribe to payment events.
    /// Returns a receiver that will receive events.
    fn subscribe(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = PayServerResult<tokio::sync::broadcast::Receiver<PaymentEvent>>>
                + Send
                + '_,
        >,
    >;
}

/// Trait for publishing payment events.
pub trait PaymentEventPublisher: Send + Sync {
    /// Publish a payment event.
    fn publish(
        &self,
        event: PaymentEvent,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_invoice_request_builder() {
        let request = CreateInvoiceRequest::new(100_000, Currency::BTC)
            .with_expiration(3600)
            .with_payment_methods(vec![PaymentMethod::BitcoinOnChain])
            .with_webhook("https://example.com/webhook".to_string());

        assert_eq!(request.amount, 100_000);
        assert_eq!(request.currency, Currency::BTC);
        assert_eq!(request.expiration_seconds, Some(3600));
        assert_eq!(request.payment_methods.len(), 1);
        assert!(request.webhook_url.is_some());
    }

    #[test]
    fn test_invoice_query_default() {
        let query = InvoiceQuery::default();
        assert!(query.status.is_none());
        assert!(query.currency.is_none());
        assert!(query.limit.is_none());
        assert!(query.offset.is_none());
    }
}
