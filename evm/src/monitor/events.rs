//! Monitor events and commands for payment detection.
//!
//! Events flow from monitor -> API server (e.g., PaymentDetected).
//! Commands flow from API server -> monitor (e.g., WatchAddress).

use alloy::primitives::{Address, B256, U256};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// Commands (API Server -> Monitor)
// ============================================================================

/// Commands sent to the monitor from the API server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MonitorCommand {
    /// Watch an address for incoming payments.
    WatchAddress(WatchAddressCommand),
    /// Stop watching an address.
    UnwatchAddress(UnwatchAddressCommand),
    /// Request current status of watched addresses.
    GetStatus(GetStatusCommand),
}

/// Command to watch an address for payments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchAddressCommand {
    /// Target chain ID.
    pub chain_id: u64,
    /// Address to watch.
    pub address: Address,
    /// Invoice ID this address is for.
    pub invoice_id: uuid::Uuid,
    /// Expected amount (optional, for validation).
    pub expected_amount: Option<U256>,
    /// Token contract address (None = native currency).
    pub token_contract: Option<Address>,
}

/// Command to stop watching an address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnwatchAddressCommand {
    /// Target chain ID.
    pub chain_id: u64,
    /// Address to stop watching.
    pub address: Address,
    /// Token contract address (None = native currency).
    pub token_contract: Option<Address>,
}

/// Request status of watched addresses on a chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetStatusCommand {
    /// Target chain ID (None = all chains).
    pub chain_id: Option<u64>,
}

// ============================================================================
// Events (Monitor -> API Server)
// ============================================================================

/// Events emitted by the payment monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MonitorEvent {
    /// A payment was detected (may be unconfirmed).
    PaymentDetected(PaymentDetected),
    /// A payment reached required confirmations.
    PaymentConfirmed(PaymentConfirmed),
    /// A chain reorganization was detected.
    ReorgDetected(ReorgDetected),
    /// Monitor started for a chain.
    MonitorStarted { chain_id: u64 },
    /// Monitor stopped for a chain.
    MonitorStopped { chain_id: u64 },
    /// Monitor encountered an error.
    MonitorError { chain_id: u64, error: String },
    /// Address was added to watch list.
    AddressWatched(AddressWatched),
    /// Address was removed from watch list.
    AddressUnwatched(AddressUnwatched),
    /// Status response (in response to GetStatus command).
    StatusReport(StatusReport),
}

/// Event confirming an address is now being watched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressWatched {
    /// Chain ID.
    pub chain_id: u64,
    /// Address being watched.
    pub address: Address,
    /// Invoice ID.
    pub invoice_id: uuid::Uuid,
    /// When watching started.
    pub watched_at: DateTime<Utc>,
}

/// Event confirming an address is no longer being watched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressUnwatched {
    /// Chain ID.
    pub chain_id: u64,
    /// Address that was unwatched.
    pub address: Address,
    /// Invoice ID (if it was being watched).
    pub invoice_id: Option<uuid::Uuid>,
}

/// Status report for monitored addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusReport {
    /// Chain ID.
    pub chain_id: u64,
    /// Number of watched addresses.
    pub watched_count: usize,
    /// Current block number.
    pub current_block: u64,
    /// Watched addresses with their invoice IDs.
    pub addresses: Vec<WatchedAddressInfo>,
    /// Report timestamp.
    pub reported_at: DateTime<Utc>,
}

/// Info about a watched address (for status reports).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedAddressInfo {
    /// The watched address.
    pub address: Address,
    /// Invoice ID.
    pub invoice_id: uuid::Uuid,
    /// Token contract (None = native).
    pub token_contract: Option<Address>,
}

/// Payment detected event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentDetected {
    /// Chain ID where payment was detected.
    pub chain_id: u64,
    /// Invoice ID this payment is for.
    pub invoice_id: uuid::Uuid,
    /// Payment address that received funds.
    pub payment_address: Address,
    /// Amount received (in smallest unit).
    pub amount: U256,
    /// Transaction hash.
    pub tx_hash: B256,
    /// Block number.
    pub block_number: u64,
    /// Block hash.
    pub block_hash: B256,
    /// Log index within the block (for ERC20).
    pub log_index: Option<u32>,
    /// Whether this is a native transfer or token transfer.
    pub is_native: bool,
    /// Token contract address (if ERC20).
    pub token_address: Option<Address>,
    /// Sender address.
    pub from_address: Address,
    /// Current confirmations.
    pub confirmations: u64,
    /// Required confirmations for this chain.
    pub required_confirmations: u64,
    /// When the payment was detected.
    pub detected_at: DateTime<Utc>,
}

impl PaymentDetected {
    /// Check if this payment has enough confirmations.
    pub fn is_confirmed(&self) -> bool {
        self.confirmations >= self.required_confirmations
    }
}

/// Payment confirmed event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentConfirmed {
    /// Chain ID.
    pub chain_id: u64,
    /// Invoice ID.
    pub invoice_id: uuid::Uuid,
    /// Payment address.
    pub payment_address: Address,
    /// Amount received.
    pub amount: U256,
    /// Transaction hash.
    pub tx_hash: B256,
    /// Block number.
    pub block_number: u64,
    /// Final confirmations count.
    pub confirmations: u64,
    /// When confirmed.
    pub confirmed_at: DateTime<Utc>,
}

/// Chain reorganization detected event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgDetected {
    /// Chain ID where reorg occurred.
    pub chain_id: u64,
    /// Block number where the fork started.
    pub fork_block: u64,
    /// Old block hash at fork point.
    pub old_hash: B256,
    /// New block hash at fork point.
    pub new_hash: B256,
    /// Depth of the reorg (number of blocks replaced).
    pub depth: u64,
    /// Invoice IDs that may be affected.
    pub affected_invoices: Vec<uuid::Uuid>,
    /// When detected.
    pub detected_at: DateTime<Utc>,
}

impl ReorgDetected {
    /// Check if this reorg is significant (deep).
    pub fn is_significant(&self, threshold: u64) -> bool {
        self.depth >= threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_detected_confirmation() {
        let payment = PaymentDetected {
            chain_id: 1,
            invoice_id: uuid::Uuid::new_v4(),
            payment_address: Address::ZERO,
            amount: U256::from(1000000),
            tx_hash: B256::ZERO,
            block_number: 100,
            block_hash: B256::ZERO,
            log_index: None,
            is_native: true,
            token_address: None,
            from_address: Address::ZERO,
            confirmations: 6,
            required_confirmations: 12,
            detected_at: Utc::now(),
        };

        assert!(!payment.is_confirmed());

        let confirmed = PaymentDetected {
            confirmations: 12,
            ..payment
        };
        assert!(confirmed.is_confirmed());
    }

    #[test]
    fn test_reorg_significance() {
        let reorg = ReorgDetected {
            chain_id: 1,
            fork_block: 100,
            old_hash: B256::ZERO,
            new_hash: B256::ZERO,
            depth: 2,
            affected_invoices: vec![],
            detected_at: Utc::now(),
        };

        assert!(!reorg.is_significant(3));
        assert!(reorg.is_significant(2));
    }
}
