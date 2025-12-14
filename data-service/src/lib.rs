//! Data access layer for EthPayServer.
//!
//! This crate provides database implementations for the repository traits
//! defined in the `types` crate.
//!
//! # Features
//!
//! - `postgres` (default) - PostgreSQL implementation
//!
//! # Implementations
//!
//! - PostgreSQL implementation (production, requires `postgres` feature)
//! - In-memory implementation (testing, requires `test-utils` feature)

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

// Re-export repository traits and types from the types crate for convenience.
pub use types::{
    // Combined traits
    DataService, DataServiceReader, DataServiceWriter,
    // Invoice
    InvoiceQueryParams, InvoiceReader, InvoiceRepository, InvoiceWriter,
    // Payment
    PaymentQueryParams, PaymentReader, PaymentRepository, PaymentWriter,
    // Watched Address
    WatchedAddressReader, WatchedAddressRepository, WatchedAddressWriter,
    // Errors
    RepositoryError, RepositoryResult,
};

/// Convert sqlx::Error to RepositoryError.
///
/// This helper is needed because we can't implement From trait due to orphan rules.
#[cfg(feature = "postgres")]
pub fn sqlx_to_repo_error(e: sqlx::Error) -> RepositoryError {
    match e {
        sqlx::Error::RowNotFound => RepositoryError::NotFound("row not found".into()),
        _ => RepositoryError::Database(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use types::{
        InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter, Network,
        WatchedAddressReader, WatchedAddressWriter,
    };

    #[tokio::test]
    async fn test_in_memory_data_service() {
        let ds = InMemoryDataService::new();
        let invoice = create_test_invoice();

        // Upsert
        InvoiceWriter::upsert(&ds, &invoice).await.unwrap();

        // Get
        let retrieved = InvoiceReader::get(&ds, &invoice.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, invoice.id);

        // Update status
        InvoiceWriter::update_status(&ds, &invoice.id, InvoiceStatus::Paid)
            .await
            .unwrap();
        let updated = InvoiceReader::get(&ds, &invoice.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn test_watched_addresses() {
        let ds = InMemoryDataService::new();
        let invoice_id = InvoiceId::new();
        let address = "0x1234567890abcdef1234567890abcdef12345678";
        let network = Network::Ethereum;

        WatchedAddressWriter::upsert(&ds, address, &invoice_id, network)
            .await
            .unwrap();

        let found = WatchedAddressReader::get_invoice_id(&ds, address, network)
            .await
            .unwrap();
        assert_eq!(found, Some(invoice_id.clone()));

        WatchedAddressWriter::remove(&ds, address, network)
            .await
            .unwrap();
        let found = WatchedAddressReader::get_invoice_id(&ds, address, network)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_query_by_network() {
        let ds = InMemoryDataService::new();

        let eth_invoice = create_test_invoice();
        InvoiceWriter::upsert(&ds, &eth_invoice).await.unwrap();

        let mut btc_invoice = create_test_invoice();
        btc_invoice.id = InvoiceId::new();
        btc_invoice.network = Network::BitcoinMainnet;
        btc_invoice.asset_symbol = "BTC".to_string();
        InvoiceWriter::upsert(&ds, &btc_invoice).await.unwrap();

        // Query all
        let params = InvoiceQueryParams::new();
        let (total, _) = InvoiceReader::query(&ds, &params).await.unwrap();
        assert_eq!(total, 2);

        // Query ETH only
        let params = InvoiceQueryParams::new().with_network(Network::Ethereum);
        let (total, invoices) = InvoiceReader::query(&ds, &params).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(invoices[0].network, Network::Ethereum);
    }
}
