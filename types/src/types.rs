//! Core types for the PayServer ecosystem.
//!
//! This module contains network-agnostic types that are shared across all PayServers.
//! Network-specific types (like ERC20 tokens, Lightning invoices) are defined in
//! their respective PayServer crates.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Supported blocknetwork networks across the PayServer ecosystem.
///
/// Each PayServer implementation supports a subset of these networks.
/// For example, `ethpayserver` handles all EVM networks, while `bitcoinpayserver`
/// handles Bitcoin mainnet and Lightning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    // =========================================================================
    // Bitcoin family
    // =========================================================================
    /// Bitcoin mainnet (on-network)
    BitcoinMainnet,
    /// Bitcoin Lightning Network
    BitcoinLightning,

    // =========================================================================
    // EVM-compatible networks
    // =========================================================================
    /// Ethereum mainnet
    Ethereum,
    /// Polygon (formerly Matic)
    Polygon,
    /// Arbitrum One
    Arbitrum,
    /// Optimism
    Optimism,
    /// Base (Coinbase L2)
    Base,
    /// Avalanche C-Network
    Avalanche,
    /// BNB Smart Network (formerly BSC)
    BinanceSmartChain,
    /// zkSync Era
    ZkSync,
    /// Linea
    Linea,
    /// Scroll
    Scroll,
}

impl Network {
    /// Returns true if this is an EVM-compatible network.
    pub fn is_evm(&self) -> bool {
        matches!(
            self,
            Network::Ethereum
                | Network::Polygon
                | Network::Arbitrum
                | Network::Optimism
                | Network::Base
                | Network::Avalanche
                | Network::BinanceSmartChain
                | Network::ZkSync
                | Network::Linea
                | Network::Scroll
        )
    }

    /// Returns true if this is a Bitcoin-family network.
    pub fn is_bitcoin(&self) -> bool {
        matches!(self, Network::BitcoinMainnet | Network::BitcoinLightning)
    }

    /// Returns the display name for this network.
    pub fn display_name(&self) -> &'static str {
        match self {
            Network::BitcoinMainnet => "Bitcoin",
            Network::BitcoinLightning => "Lightning Network",
            Network::Ethereum => "Ethereum",
            Network::Polygon => "Polygon",
            Network::Arbitrum => "Arbitrum",
            Network::Optimism => "Optimism",
            Network::Base => "Base",
            Network::Avalanche => "Avalanche",
            Network::BinanceSmartChain => "BNB Network",
            Network::ZkSync => "zkSync",
            Network::Linea => "Linea",
            Network::Scroll => "Scroll",
        }
    }

    /// Returns the native currency symbol for this network.
    pub fn native_symbol(&self) -> &'static str {
        match self {
            Network::BitcoinMainnet | Network::BitcoinLightning => "BTC",
            Network::Ethereum => "ETH",
            Network::Polygon => "POL",
            Network::Arbitrum => "ETH",
            Network::Optimism => "ETH",
            Network::Base => "ETH",
            Network::Avalanche => "AVAX",
            Network::BinanceSmartChain => "BNB",
            Network::ZkSync => "ETH",
            Network::Linea => "ETH",
            Network::Scroll => "ETH",
        }
    }

    /// Returns the network identifier as a string (for database storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BitcoinMainnet => "bitcoin_mainnet",
            Self::BitcoinLightning => "bitcoin_lightning",
            Self::Ethereum => "ethereum",
            Self::Polygon => "polygon",
            Self::Arbitrum => "arbitrum",
            Self::Optimism => "optimism",
            Self::Base => "base",
            Self::Avalanche => "avalanche",
            Self::BinanceSmartChain => "binance_smart_chain",
            Self::ZkSync => "zksync",
            Self::Linea => "linea",
            Self::Scroll => "scroll",
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Network {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bitcoin_mainnet" | "bitcoin" | "btc" => Ok(Self::BitcoinMainnet),
            "bitcoin_lightning" | "lightning" | "ln" => Ok(Self::BitcoinLightning),
            "ethereum" | "eth" => Ok(Self::Ethereum),
            "polygon" | "matic" => Ok(Self::Polygon),
            "arbitrum" | "arb" => Ok(Self::Arbitrum),
            "optimism" | "op" => Ok(Self::Optimism),
            "base" => Ok(Self::Base),
            "avalanche" | "avax" => Ok(Self::Avalanche),
            "binance_smart_chain" | "bsc" | "bnb" => Ok(Self::BinanceSmartChain),
            "zksync" | "zk_sync" => Ok(Self::ZkSync),
            "linea" => Ok(Self::Linea),
            "scroll" => Ok(Self::Scroll),
            _ => Err(format!("unknown network: {}", s)),
        }
    }
}

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UserId(pub Uuid);

impl UserId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
    /// Returns true if this is a final status (no more changes expected).
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            InvoiceStatus::Paid
                | InvoiceStatus::Expired
                | InvoiceStatus::Cancelled
                | InvoiceStatus::Refunded
        )
    }

    /// Returns true if this invoice can still receive payments.
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

impl std::str::FromStr for InvoiceStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(InvoiceStatus::Pending),
            "processing" => Ok(InvoiceStatus::Processing),
            "partially_paid" => Ok(InvoiceStatus::PartiallyPaid),
            "paid" => Ok(InvoiceStatus::Paid),
            "expired" => Ok(InvoiceStatus::Expired),
            "cancelled" | "canceled" => Ok(InvoiceStatus::Cancelled),
            "refunded" => Ok(InvoiceStatus::Refunded),
            _ => Err(format!("unknown invoice status: {}", s)),
        }
    }
}

/// Health status of a PayServer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Whether the service is healthy.
    pub healthy: bool,
    /// Service version.
    pub version: String,
    /// Networks this server supports.
    pub supported_networks: Vec<Network>,
    /// Current block heights per network (if applicable).
    pub block_heights: Option<std::collections::HashMap<Network, u64>>,
    /// Number of pending invoices.
    pub pending_invoices: Option<u64>,
    /// Additional details.
    pub details: Option<serde_json::Value>,
}

/// Events emitted by the payment system.
///
/// These are network-agnostic events. PayServers may emit additional
/// network-specific events internally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PaymentEvent {
    /// Invoice was created.
    InvoiceCreated {
        invoice_id: InvoiceId,
        network: Network,
    },
    /// Payment was detected (unconfirmed).
    PaymentDetected {
        invoice_id: InvoiceId,
        tx_hash: String,
        network: Network,
    },
    /// Payment was confirmed.
    PaymentConfirmed {
        invoice_id: InvoiceId,
        tx_hash: String,
        confirmations: u32,
    },
    /// Invoice status changed.
    InvoiceStatusChanged {
        invoice_id: InvoiceId,
        old_status: InvoiceStatus,
        new_status: InvoiceStatus,
    },
    /// Invoice was fully paid.
    InvoicePaid { invoice_id: InvoiceId },
    /// Invoice expired.
    InvoiceExpired { invoice_id: InvoiceId },
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
    fn test_network_is_evm() {
        assert!(Network::Ethereum.is_evm());
        assert!(Network::Polygon.is_evm());
        assert!(Network::Arbitrum.is_evm());
        assert!(!Network::BitcoinMainnet.is_evm());
        assert!(!Network::BitcoinLightning.is_evm());
    }

    #[test]
    fn test_network_is_bitcoin() {
        assert!(Network::BitcoinMainnet.is_bitcoin());
        assert!(Network::BitcoinLightning.is_bitcoin());
        assert!(!Network::Ethereum.is_bitcoin());
    }

    #[test]
    fn test_network_native_symbol() {
        assert_eq!(Network::BitcoinMainnet.native_symbol(), "BTC");
        assert_eq!(Network::Ethereum.native_symbol(), "ETH");
        assert_eq!(Network::Polygon.native_symbol(), "POL");
        assert_eq!(Network::Avalanche.native_symbol(), "AVAX");
        assert_eq!(Network::BinanceSmartChain.native_symbol(), "BNB");
    }

    #[test]
    fn test_invoice_status() {
        assert!(InvoiceStatus::Paid.is_final());
        assert!(InvoiceStatus::Expired.is_final());
        assert!(!InvoiceStatus::Pending.is_final());
        assert!(!InvoiceStatus::Processing.is_final());

        assert!(InvoiceStatus::Pending.is_payable());
        assert!(InvoiceStatus::PartiallyPaid.is_payable());
        assert!(!InvoiceStatus::Paid.is_payable());
        assert!(!InvoiceStatus::Expired.is_payable());
    }

    #[test]
    fn test_network_display() {
        assert_eq!(Network::Ethereum.to_string(), "ethereum");
        assert_eq!(Network::BitcoinLightning.to_string(), "bitcoin_lightning");
        assert_eq!(Network::BinanceSmartChain.to_string(), "binance_smart_chain");
    }

    #[test]
    fn test_network_from_str() {
        assert_eq!("ethereum".parse::<Network>().unwrap(), Network::Ethereum);
        assert_eq!("eth".parse::<Network>().unwrap(), Network::Ethereum);
        assert_eq!("polygon".parse::<Network>().unwrap(), Network::Polygon);
        assert_eq!("bsc".parse::<Network>().unwrap(), Network::BinanceSmartChain);
        assert!("invalid".parse::<Network>().is_err());
    }
}
