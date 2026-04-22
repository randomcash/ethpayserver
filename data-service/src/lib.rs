//! Data access layer for EthPayServer.
//!
//! This crate provides database implementations for the repository traits
//! defined in the `types` crate.
//!
//! # Features
//!
//! - `postgres` (default) - PostgreSQL implementation
//! - `test-utils` - In-memory implementation for testing
//!
//! # Migrations
//!
//! Each database implementation has its own migrations directory:
//!
//! ```bash
//! # PostgreSQL
//! sqlx migrate run --source migrations/postgres
//! ```

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "postgres")]
pub use postgres::{ApiKeyRateLimitInfo, PendingWatch, PgDataService};

#[cfg(feature = "redis")]
pub mod redis;

#[cfg(feature = "redis")]
pub use redis::RedisDataService;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

#[cfg(any(test, feature = "test-utils"))]
pub use test_utils::InMemoryDataService;

// Re-export repository traits and types from the types crate for convenience.
pub use types::{
    // Watched Address (for PostgreSQL persistence)
    CleanupAddressInfo,
    // Combined traits
    DataService,
    DataServiceReader,
    DataServiceWriter,
    // Invoice
    InvoiceQueryParams,
    InvoiceReader,
    InvoiceRepository,
    InvoiceWriter,
    // Live Watched Address (for evmmonitor/Redis)
    LiveWatchedAddressReader,
    LiveWatchedAddressRepository,
    LiveWatchedAddressWriter,
    // Payment Event
    PaymentEventWriter,
    // Payment Option
    PaymentMethodId,
    PaymentOptionData,
    PaymentOptionId,
    PaymentOptionReader,
    PaymentOptionRepository,
    PaymentOptionWriter,
    // Payment
    PaymentQueryParams,
    PaymentReader,
    PaymentRepository,
    PaymentWriter,
    // Payout
    PayoutData,
    PayoutReader,
    PayoutRepository,
    PayoutStatus,
    PayoutWriter,
    PendingWatchInfo,
    // Refund
    RefundData,
    RefundReader,
    RefundRepository,
    RefundStatus,
    RefundWriter,
    // Errors
    RepositoryError,
    RepositoryResult,
    // Store Payment Method
    StorePaymentMethod,
    StorePaymentMethodReader,
    StorePaymentMethodRepository,
    StorePaymentMethodWriter,
    // Store Settings
    StoreSettings,
    StoreSettingsReader,
    StoreSettingsRepository,
    StoreSettingsWriter,
    // Store Wallet (deprecated)
    StoreWallet,
    StoreWalletReader,
    StoreWalletRepository,
    StoreWalletWriter,
    // Store Webhook
    StoreWebhook,
    StoreWebhookReader,
    StoreWebhookRepository,
    StoreWebhookWriter,
    // Token
    TokenData,
    TokenQueryParams,
    TokenReader,
    TokenRepository,
    TokenWriter,
    WatchedAddressReader,
    WatchedAddressRepository,
    WatchedAddressWriter,
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
        InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus, InvoiceWriter,
        PaymentOptionId, WatchedAddressReader, WatchedAddressWriter,
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
        let updated = InvoiceReader::get(&ds, &invoice.id).await.unwrap().unwrap();
        assert_eq!(updated.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn test_watched_addresses() {
        let ds = InMemoryDataService::new();
        let payment_option_id = PaymentOptionId(uuid::Uuid::new_v4());
        let address = "0x1234567890abcdef1234567890abcdef12345678";
        let chain_id: u64 = 1; // Ethereum mainnet

        WatchedAddressWriter::upsert(&ds, address, &payment_option_id, chain_id, None)
            .await
            .unwrap();

        let found = WatchedAddressReader::get_payment_option_id(&ds, address, chain_id, None)
            .await
            .unwrap();
        assert_eq!(found, Some(payment_option_id.clone()));

        WatchedAddressWriter::deactivate(&ds, address, chain_id, None)
            .await
            .unwrap();
        let found = WatchedAddressReader::get_payment_option_id(&ds, address, chain_id, None)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_query_by_currency() {
        let ds = InMemoryDataService::new();

        let usd_invoice = create_test_invoice();
        InvoiceWriter::upsert(&ds, &usd_invoice).await.unwrap();

        let mut eur_invoice = create_test_invoice();
        eur_invoice.id = InvoiceId::new();
        eur_invoice.currency = "EUR".to_string();
        InvoiceWriter::upsert(&ds, &eur_invoice).await.unwrap();

        // Query all
        let params = InvoiceQueryParams::new();
        let (total, _) = InvoiceReader::query(&ds, &params).await.unwrap();
        assert_eq!(total, 2);

        // Query USD only
        let params = InvoiceQueryParams::new().with_currency("USD");
        let (total, invoices) = InvoiceReader::query(&ds, &params).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(invoices[0].currency, "USD");
    }
}
