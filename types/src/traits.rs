//! Core traits for the PayServer ecosystem.
//!
//! These traits define the interface that all PayServer implementations must follow.
//! Each PayServer (ethpayserver, bitcoinpayserver, etc.) implements these traits
//! with their network-specific logic.

use std::future::Future;
use std::pin::Pin;

use crate::error::PayServerResult;
use crate::store::StoreId;
use crate::types::{HealthStatus, InvoiceId, InvoiceStatus, Network, PaymentEvent};

/// Configuration for creating an invoice.
///
/// This is a generic request structure. Each PayServer interprets
/// the `asset_details` field according to its supported networks.
#[derive(Debug, Clone)]
pub struct CreateInvoiceRequest {
    /// The store this invoice belongs to.
    pub store_id: StoreId,
    /// The blocknetwork to receive payment on.
    pub network: Network,
    /// Amount in the smallest unit (satoshis, wei, etc.) as a string to support large values.
    pub amount: String,
    /// Asset-specific details (e.g., token contract address for ERC20).
    /// For native assets, this can be None.
    pub asset_details: Option<serde_json::Value>,
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
    /// Create a new invoice request for native currency on a network.
    pub fn native(store_id: StoreId, network: Network, amount: impl Into<String>) -> Self {
        Self {
            store_id,
            network,
            amount: amount.into(),
            asset_details: None,
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

    pub fn with_asset_details(mut self, details: serde_json::Value) -> Self {
        self.asset_details = Some(details);
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
    /// Filter by network.
    pub network: Option<Network>,
    /// Maximum number of results.
    pub limit: Option<u32>,
    /// Offset for pagination.
    pub offset: Option<u32>,
}

/// Generic invoice data returned by PayServers.
///
/// This contains the common fields. PayServers may include additional
/// network-specific data in the `extra` field.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvoiceData {
    pub id: InvoiceId,
    /// The store this invoice belongs to.
    pub store_id: StoreId,
    pub network: Network,
    pub status: InvoiceStatus,
    /// Amount requested (smallest unit as string).
    pub amount: String,
    /// Amount received so far (smallest unit as string).
    pub amount_received: String,
    /// Asset symbol (e.g., "ETH", "BTC", "USDT").
    pub asset_symbol: String,
    /// Payment address.
    pub payment_address: Option<String>,
    /// Payment request string (e.g., Lightning invoice, EIP-681 URI).
    pub payment_request: Option<String>,
    /// When the invoice was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the invoice expires.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
    /// Network-specific extra data.
    pub extra: Option<serde_json::Value>,
}

/// Generic payment data returned by PayServers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaymentData {
    pub id: uuid::Uuid,
    pub invoice_id: InvoiceId,
    pub network: Network,
    /// Amount received (smallest unit as string).
    pub amount: String,
    /// Asset symbol.
    pub asset_symbol: String,
    /// Transaction hash.
    pub tx_hash: String,
    /// Block number (if confirmed).
    pub block_number: Option<u64>,
    /// Number of confirmations.
    pub confirmations: u32,
    /// When the payment was detected.
    pub detected_at: chrono::DateTime<chrono::Utc>,
    /// When the payment was confirmed.
    pub confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Sender address (if known).
    pub from_address: Option<String>,
    /// Network-specific extra data.
    pub extra: Option<serde_json::Value>,
}

/// Core trait that all payment servers must implement.
pub trait PayServer: Send + Sync {
    /// Returns the networks this PayServer supports.
    fn supported_networks(&self) -> Vec<Network>;

    /// Create a new invoice.
    fn create_invoice(
        &self,
        request: CreateInvoiceRequest,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<InvoiceData>> + Send + '_>>;

    /// Get an invoice by ID.
    fn get_invoice(
        &self,
        id: &InvoiceId,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<InvoiceData>> + Send + '_>>;

    /// Cancel an invoice.
    fn cancel_invoice(
        &self,
        id: &InvoiceId,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;

    /// List invoices matching the query.
    fn list_invoices(
        &self,
        query: InvoiceQuery,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<Vec<InvoiceData>>> + Send + '_>>;

    /// Get all payments for an invoice.
    fn get_payments(
        &self,
        invoice_id: &InvoiceId,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<Vec<PaymentData>>> + Send + '_>>;

    /// Get health status of the payment server.
    fn health(&self) -> Pin<Box<dyn Future<Output = PayServerResult<HealthStatus>> + Send + '_>>;
}

/// Trait for monitoring blocknetwork for payments.
pub trait PaymentMonitor: Send + Sync {
    /// Start monitoring for payments.
    fn start(&self) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;

    /// Stop monitoring.
    fn stop(&self) -> Pin<Box<dyn Future<Output = PayServerResult<()>> + Send + '_>>;

    /// Check if the monitor is running.
    fn is_running(&self) -> bool;

    /// Get the current block height being monitored for a network.
    fn current_block_height(
        &self,
        network: Network,
    ) -> Pin<Box<dyn Future<Output = PayServerResult<u64>> + Send + '_>>;
}

/// Trait for subscribing to payment events.
pub trait PaymentEventSubscriber: Send + Sync {
    /// Subscribe to payment events.
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
        let store_id = StoreId::new();
        let request = CreateInvoiceRequest::native(store_id, Network::Ethereum, "1000000000000000000")
            .with_expiration(3600)
            .with_webhook("https://example.com/webhook".to_string());

        assert_eq!(request.store_id, store_id);
        assert_eq!(request.network, Network::Ethereum);
        assert_eq!(request.amount, "1000000000000000000");
        assert_eq!(request.expiration_seconds, Some(3600));
        assert!(request.asset_details.is_none());
        assert!(request.webhook_url.is_some());
    }

    #[test]
    fn test_create_invoice_request_with_token() {
        let store_id = StoreId::new();
        let token_details = serde_json::json!({
            "contract_address": "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            "symbol": "USDT",
            "decimals": 6
        });

        let request = CreateInvoiceRequest::native(store_id, Network::Ethereum, "1000000")
            .with_asset_details(token_details);

        assert!(request.asset_details.is_some());
    }

    #[test]
    fn test_invoice_query_default() {
        let query = InvoiceQuery::default();
        assert!(query.status.is_none());
        assert!(query.network.is_none());
        assert!(query.limit.is_none());
        assert!(query.offset.is_none());
    }
}
