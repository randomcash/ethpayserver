//! Database type conversions.

use crate::RepositoryError;
use types::{InvoiceStatus, Network};

/// Convert Rust Network enum to database string.
///
/// Returns an error if given a non-EVM network (e.g., Bitcoin networks).
/// Ethpayserver only handles EVM-compatible chains.
pub fn try_network_to_db(network: Network) -> Result<&'static str, RepositoryError> {
    match network {
        Network::Ethereum => Ok("ethereum"),
        Network::Polygon => Ok("polygon"),
        Network::Arbitrum => Ok("arbitrum"),
        Network::Optimism => Ok("optimism"),
        Network::Base => Ok("base"),
        Network::Avalanche => Ok("avalanche"),
        Network::BinanceSmartChain => Ok("binance_smart_chain"),
        Network::ZkSync => Ok("zksync"),
        Network::Linea => Ok("linea"),
        Network::Scroll => Ok("scroll"),
        _ => Err(RepositoryError::InvalidData(format!(
            "Unsupported network for ethpayserver: {:?}",
            network
        ))),
    }
}

/// Convert database string to Rust Network enum.
///
/// Returns an error if the database contains an unknown network value,
/// indicating data corruption or schema mismatch.
pub fn try_db_to_network(s: &str) -> Result<Network, RepositoryError> {
    match s {
        "ethereum" => Ok(Network::Ethereum),
        "polygon" => Ok(Network::Polygon),
        "arbitrum" => Ok(Network::Arbitrum),
        "optimism" => Ok(Network::Optimism),
        "base" => Ok(Network::Base),
        "avalanche" => Ok(Network::Avalanche),
        "binance_smart_chain" => Ok(Network::BinanceSmartChain),
        "zksync" => Ok(Network::ZkSync),
        "linea" => Ok(Network::Linea),
        "scroll" => Ok(Network::Scroll),
        _ => Err(RepositoryError::InvalidData(format!(
            "Unknown network in database: {}",
            s
        ))),
    }
}

/// Convert Rust InvoiceStatus enum to database string.
///
/// This is infallible since all InvoiceStatus variants are supported.
pub fn status_to_db(status: InvoiceStatus) -> &'static str {
    match status {
        InvoiceStatus::Pending => "pending",
        InvoiceStatus::Processing => "processing",
        InvoiceStatus::PartiallyPaid => "partially_paid",
        InvoiceStatus::LatePaid => "late_paid",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::Expired => "expired",
        InvoiceStatus::Cancelled => "cancelled",
        InvoiceStatus::Refunded => "refunded",
    }
}

/// Convert database string to Rust InvoiceStatus enum.
///
/// Returns an error if the database contains an unknown status value,
/// indicating data corruption or schema mismatch.
pub fn try_db_to_status(s: &str) -> Result<InvoiceStatus, RepositoryError> {
    match s {
        "pending" => Ok(InvoiceStatus::Pending),
        "processing" => Ok(InvoiceStatus::Processing),
        "partially_paid" => Ok(InvoiceStatus::PartiallyPaid),
        "late_paid" => Ok(InvoiceStatus::LatePaid),
        "paid" => Ok(InvoiceStatus::Paid),
        "expired" => Ok(InvoiceStatus::Expired),
        "cancelled" => Ok(InvoiceStatus::Cancelled),
        "refunded" => Ok(InvoiceStatus::Refunded),
        _ => Err(RepositoryError::InvalidData(format!(
            "Unknown invoice status in database: {}",
            s
        ))),
    }
}
