//! Configuration and watched-address types for the chain monitor.

use crate::network::ChainConfig;
use alloy::primitives::{Address, U256};

/// Configuration for the chain monitor.
#[derive(Debug, Clone)]
pub struct ChainMonitorConfig {
    /// Number of confirmations required.
    pub required_confirmations: u64,
    /// Maximum blocks to scan per iteration.
    pub max_blocks_per_scan: u64,
    /// How often to check pending payments (seconds).
    pub confirmation_check_interval_secs: u64,
    /// Whether to detect native (ETH) transfers.
    pub monitor_native: bool,
    /// Whether to detect ERC20 transfers.
    pub monitor_erc20: bool,
}

impl Default for ChainMonitorConfig {
    fn default() -> Self {
        Self {
            required_confirmations: 12,
            max_blocks_per_scan: 100,
            confirmation_check_interval_secs: 30,
            monitor_native: true,
            monitor_erc20: true,
        }
    }
}

impl ChainMonitorConfig {
    /// Create config from chain defaults.
    pub fn from_chain(chain: &ChainConfig) -> Self {
        Self {
            required_confirmations: chain.confirmations_required as u64,
            ..Default::default()
        }
    }
}

/// An address being watched for payments.
#[derive(Debug, Clone)]
pub struct WatchedAddress {
    /// The address to watch.
    pub address: Address,
    /// Invoice ID this address is for.
    pub invoice_id: uuid::Uuid,
    /// Expected amount (if known).
    pub expected_amount: Option<U256>,
    /// Token contract (None = native).
    pub token_contract: Option<Address>,
    /// When watching started.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Previous known balance (for native).
    pub last_known_balance: U256,
}
