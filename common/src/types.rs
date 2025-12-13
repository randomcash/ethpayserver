//! Core types for the PayServer ecosystem.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an invoice.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvoiceId(pub String);

impl InvoiceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for InvoiceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for InvoiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a monetary amount with currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Amount {
    /// The value in the smallest unit of the currency (e.g., satoshis for BTC, wei for ETH).
    pub value: i64,
    /// The currency of the amount.
    pub currency: Currency,
}

impl Amount {
    pub fn new(value: i64, currency: Currency) -> Self {
        Self { value, currency }
    }

    /// Create an amount in BTC (value in satoshis).
    pub fn btc(satoshis: i64) -> Self {
        Self::new(satoshis, Currency::BTC)
    }

    /// Create an amount in ETH (value in wei).
    pub fn eth(wei: i64) -> Self {
        Self::new(wei, Currency::ETH)
    }

    /// Create an amount in USDT (value in smallest unit, 6 decimals).
    pub fn usdt(value: i64) -> Self {
        Self::new(value, Currency::USDT)
    }

    /// Convert to decimal representation using the currency's decimals.
    pub fn to_decimal(&self) -> Decimal {
        let decimals = self.currency.decimals();
        Decimal::new(self.value, decimals as u32)
    }
}

/// Supported currencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// Bitcoin
    BTC,
    /// Ethereum
    ETH,
    /// Tether USD (ERC-20)
    USDT,
    /// USD Coin (ERC-20)
    USDC,
    /// Litecoin
    LTC,
}

impl Currency {
    /// Returns the number of decimal places for this currency.
    pub fn decimals(&self) -> u8 {
        match self {
            Currency::BTC => 8,  // satoshis
            Currency::ETH => 18, // wei
            Currency::USDT => 6,
            Currency::USDC => 6,
            Currency::LTC => 8,
        }
    }

    /// Returns the currency symbol.
    pub fn symbol(&self) -> &'static str {
        match self {
            Currency::BTC => "BTC",
            Currency::ETH => "ETH",
            Currency::USDT => "USDT",
            Currency::USDC => "USDC",
            Currency::LTC => "LTC",
        }
    }
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

/// Payment method specifying the network/protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "network")]
pub enum PaymentMethod {
    /// On-chain Bitcoin payment.
    BitcoinOnChain,
    /// Bitcoin Lightning Network payment.
    BitcoinLightning,
    /// On-chain Ethereum payment (native ETH).
    EthereumOnChain,
    /// ERC-20 token payment on Ethereum.
    ERC20 { token: Currency },
    /// On-chain Litecoin payment.
    LitecoinOnChain,
}

impl PaymentMethod {
    /// Returns the base currency for this payment method.
    pub fn currency(&self) -> Currency {
        match self {
            PaymentMethod::BitcoinOnChain | PaymentMethod::BitcoinLightning => Currency::BTC,
            PaymentMethod::EthereumOnChain => Currency::ETH,
            PaymentMethod::ERC20 { token } => *token,
            PaymentMethod::LitecoinOnChain => Currency::LTC,
        }
    }
}

/// Status of an invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    /// Invoice created, awaiting payment.
    Pending,
    /// Payment detected but not confirmed.
    Processing,
    /// Payment partially received.
    PartiallyPaid,
    /// Payment fully received and confirmed.
    Paid,
    /// Invoice expired without payment.
    Expired,
    /// Invoice cancelled.
    Cancelled,
    /// Payment refunded.
    Refunded,
}

impl InvoiceStatus {
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            InvoiceStatus::Paid
                | InvoiceStatus::Expired
                | InvoiceStatus::Cancelled
                | InvoiceStatus::Refunded
        )
    }

    pub fn is_payable(&self) -> bool {
        matches!(
            self,
            InvoiceStatus::Pending | InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid
        )
    }
}

impl std::fmt::Display for InvoiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            InvoiceStatus::Pending => "pending",
            InvoiceStatus::Processing => "processing",
            InvoiceStatus::PartiallyPaid => "partially_paid",
            InvoiceStatus::Paid => "paid",
            InvoiceStatus::Expired => "expired",
            InvoiceStatus::Cancelled => "cancelled",
            InvoiceStatus::Refunded => "refunded",
        };
        write!(f, "{}", s)
    }
}

/// An invoice requesting payment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    /// Unique identifier for this invoice.
    pub id: InvoiceId,
    /// Amount requested.
    pub amount: Amount,
    /// Accepted payment methods.
    pub payment_methods: Vec<PaymentMethod>,
    /// Current status.
    pub status: InvoiceStatus,
    /// Address to receive payment (depends on payment method).
    pub payment_address: Option<String>,
    /// Payment request string (e.g., Lightning invoice, payment URI).
    pub payment_request: Option<String>,
    /// When the invoice was created.
    pub created_at: DateTime<Utc>,
    /// When the invoice expires.
    pub expires_at: DateTime<Utc>,
    /// Amount received so far.
    pub amount_received: i64,
    /// Optional metadata.
    pub metadata: Option<serde_json::Value>,
    /// Optional webhook URL for notifications.
    pub webhook_url: Option<String>,
    /// Optional redirect URL after payment.
    pub redirect_url: Option<String>,
}

impl Invoice {
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn is_fully_paid(&self) -> bool {
        self.amount_received >= self.amount.value
    }

    pub fn remaining_amount(&self) -> i64 {
        (self.amount.value - self.amount_received).max(0)
    }
}

/// A payment received for an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    /// Unique identifier for this payment.
    pub id: Uuid,
    /// The invoice this payment is for.
    pub invoice_id: InvoiceId,
    /// Amount received.
    pub amount: Amount,
    /// Payment method used.
    pub payment_method: PaymentMethod,
    /// Transaction hash/ID on the blockchain.
    pub tx_hash: Option<String>,
    /// Number of confirmations.
    pub confirmations: u32,
    /// When the payment was detected.
    pub detected_at: DateTime<Utc>,
    /// When the payment was confirmed.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// Sender address (if known).
    pub from_address: Option<String>,
}

/// Events emitted by the payment system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum PaymentEvent {
    /// Invoice was created.
    InvoiceCreated { invoice: Invoice },
    /// Payment was detected (unconfirmed).
    PaymentDetected { payment: Payment },
    /// Payment was confirmed.
    PaymentConfirmed { payment: Payment },
    /// Invoice status changed.
    InvoiceStatusChanged {
        invoice_id: InvoiceId,
        old_status: InvoiceStatus,
        new_status: InvoiceStatus,
    },
    /// Invoice was fully paid.
    InvoicePaid { invoice: Invoice },
    /// Invoice expired.
    InvoiceExpired { invoice_id: InvoiceId },
}

/// Health status of a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Whether the service is healthy.
    pub healthy: bool,
    /// Service version.
    pub version: String,
    /// Current block height (if applicable).
    pub block_height: Option<u64>,
    /// Number of pending invoices.
    pub pending_invoices: Option<u64>,
    /// Additional details.
    pub details: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoice_id_generation() {
        let id1 = InvoiceId::new();
        let id2 = InvoiceId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_amount_to_decimal() {
        let btc = Amount::btc(100_000_000); // 1 BTC
        assert_eq!(btc.to_decimal(), Decimal::new(1, 0));

        let eth = Amount::eth(1_000_000_000_000_000_000); // 1 ETH
        assert_eq!(eth.to_decimal(), Decimal::new(1, 0));
    }

    #[test]
    fn test_currency_decimals() {
        assert_eq!(Currency::BTC.decimals(), 8);
        assert_eq!(Currency::ETH.decimals(), 18);
        assert_eq!(Currency::USDT.decimals(), 6);
    }

    #[test]
    fn test_invoice_status() {
        assert!(InvoiceStatus::Paid.is_final());
        assert!(!InvoiceStatus::Pending.is_final());
        assert!(InvoiceStatus::Pending.is_payable());
        assert!(!InvoiceStatus::Paid.is_payable());
    }

    #[test]
    fn test_payment_method_currency() {
        assert_eq!(PaymentMethod::BitcoinOnChain.currency(), Currency::BTC);
        assert_eq!(PaymentMethod::EthereumOnChain.currency(), Currency::ETH);
        assert_eq!(
            PaymentMethod::ERC20 {
                token: Currency::USDT
            }
            .currency(),
            Currency::USDT
        );
    }
}
