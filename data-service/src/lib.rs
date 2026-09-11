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

pub mod account_deletion;
pub mod analytics;
pub mod invoice_creation;

pub use account_deletion::{AccountDeletionBlockers, AccountDeletionReader};
pub use analytics::{PaymentAnalyticsReader, PaymentVolumeBucket, PaymentVolumeQuery};
pub use invoice_creation::InvoiceCreationWriter;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "postgres")]
pub use postgres::{
    ApiKeyAuthInfo, ApiKeyFullInfo, ApiKeyRateLimitInfo, PendingWatch, PgDataService,
    WalletRotation,
};

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
    // Store Token Policy
    StoreTokenPolicyEntry,
    StoreTokenPolicyReader,
    StoreTokenPolicyRepository,
    StoreTokenPolicyWithEntries,
    StoreTokenPolicyWriter,
    // Store Webhook
    StoreWebhook,
    StoreWebhookReader,
    StoreWebhookRepository,
    StoreWebhookWriter,
    // Token
    TokenData,
    TokenPolicyEntryInput,
    TokenPolicyMode,
    TokenQueryParams,
    TokenReader,
    TokenRepository,
    TokenWriter,
    // Account Wallet
    Wallet,
    WalletReader,
    WalletRepository,
    WalletWriter,
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
        PaymentOptionId, PaymentQueryParams, PaymentReader, PaymentWriter, StoreId,
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
        let updated = InvoiceReader::get(&ds, &invoice.id).await.unwrap().unwrap();
        assert_eq!(updated.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn test_watched_addresses() {
        let ds = InMemoryDataService::new();
        let payment_option_id = PaymentOptionId(uuid::Uuid::new_v4());
        let address = "0x1234567890abcdef1234567890abcdef12345678";
        let chain_id = types::ChainId::evm(1); // Ethereum mainnet

        WatchedAddressWriter::upsert(&ds, address, &payment_option_id, &chain_id, None)
            .await
            .unwrap();

        let found = WatchedAddressReader::get_payment_option_id(&ds, address, &chain_id, None)
            .await
            .unwrap();
        assert_eq!(found, Some(payment_option_id.clone()));

        WatchedAddressWriter::deactivate(&ds, address, &chain_id, None)
            .await
            .unwrap();
        let found = WatchedAddressReader::get_payment_option_id(&ds, address, &chain_id, None)
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

    // =====================================================================
    // List search
    //
    // The double has to answer these exactly as Postgres does; the same
    // assertions run against a real database in
    // `postgres::integration_tests::{invoice,payment}`.
    // =====================================================================

    #[tokio::test]
    async fn invoice_search_filters_the_count_and_the_page_together() {
        let ds = InMemoryDataService::new();

        let mut usd = create_test_invoice();
        usd.currency = "USD".to_string();
        let mut eur = create_test_invoice();
        eur.id = InvoiceId::new();
        eur.currency = "EUR".to_string();
        for inv in [&usd, &eur] {
            InvoiceWriter::upsert(&ds, inv).await.unwrap();
        }

        let (total, rows) =
            InvoiceReader::query(&ds, &InvoiceQueryParams::new().with_search("eur"))
                .await
                .unwrap();

        assert_eq!(
            rows.len(),
            1,
            "only the matching invoice belongs on the page"
        );
        assert_eq!(rows[0].id, eur.id);
        assert_eq!(
            total, 1,
            "the count must come from the same predicate as the page, or the \
             pager reports rows the search excluded"
        );
    }

    #[tokio::test]
    async fn blank_invoice_search_is_no_filter_not_an_empty_page() {
        let ds = InMemoryDataService::new();
        InvoiceWriter::upsert(&ds, &create_test_invoice())
            .await
            .unwrap();

        for blank in ["", "   "] {
            let (total, rows) =
                InvoiceReader::query(&ds, &InvoiceQueryParams::new().with_search(blank))
                    .await
                    .unwrap();
            assert_eq!(total, 1, "blank search {blank:?} must not filter");
            assert_eq!(rows.len(), 1, "blank search {blank:?} must not filter");
        }
    }

    #[tokio::test]
    async fn invoice_search_is_anded_onto_the_store_scope_never_replacing_it() {
        let ds = InMemoryDataService::new();
        let mine = StoreId::new();
        let theirs = StoreId::new();

        // The same term matches a row in each store. Only one of them is the
        // caller's, and a filter that widened the scope would return both -
        // that is a cross-store leak by another route.
        let mut ours = create_test_invoice();
        ours.store_id = mine;
        ours.currency = "USDC".to_string();
        let mut other_tenant = create_test_invoice();
        other_tenant.id = InvoiceId::new();
        other_tenant.store_id = theirs;
        other_tenant.currency = "USDC".to_string();
        for inv in [&ours, &other_tenant] {
            InvoiceWriter::upsert(&ds, inv).await.unwrap();
        }

        for scoped in [
            InvoiceQueryParams::new()
                .with_store_id(mine)
                .with_search("usdc"),
            InvoiceQueryParams::new()
                .with_store_ids(vec![mine])
                .with_search("usdc"),
        ] {
            let (total, rows) = InvoiceReader::query(&ds, &scoped).await.unwrap();
            assert_eq!(total, 1, "search must not widen the store scope");
            assert_eq!(
                rows[0].id, ours.id,
                "another store's match must stay hidden"
            );
        }
    }

    #[tokio::test]
    async fn invoice_id_search_is_anchored_and_metadata_is_not() {
        let ds = InMemoryDataService::new();
        let mut invoice = create_test_invoice();
        invoice.metadata = Some(serde_json::json!({"order_number": "SO-4471"}));
        InvoiceWriter::upsert(&ds, &invoice).await.unwrap();

        let id = invoice.id.0.clone();
        let prefix = &id[..8];
        let (total, _) = InvoiceReader::query(&ds, &InvoiceQueryParams::new().with_search(prefix))
            .await
            .unwrap();
        assert_eq!(total, 1, "an id prefix must match, upper or lower case");

        let middle = &id[4..12];
        let (total, _) = InvoiceReader::query(&ds, &InvoiceQueryParams::new().with_search(middle))
            .await
            .unwrap();
        assert_eq!(
            total, 0,
            "the id predicate is anchored, so a mid-string run must not match"
        );

        // TODO: this assertion goes away once metadata is encrypted client-side.
        let (total, _) = InvoiceReader::query(&ds, &InvoiceQueryParams::new().with_search("so-44"))
            .await
            .unwrap();
        assert_eq!(total, 1, "metadata is matched as a substring");
    }

    #[tokio::test]
    async fn payment_search_covers_hash_symbol_and_sender_and_respects_scope() {
        let ds = InMemoryDataService::new();
        let mine = StoreId::new();
        let theirs = StoreId::new();

        let mut ours = create_test_invoice();
        ours.store_id = mine;
        let mut other_tenant = create_test_invoice();
        other_tenant.id = InvoiceId::new();
        other_tenant.store_id = theirs;
        for inv in [&ours, &other_tenant] {
            InvoiceWriter::upsert(&ds, inv).await.unwrap();
        }

        // Same sender and asset on both sides of the tenant boundary.
        let mut ours_payment = create_test_payment(&ours.id, None);
        ours_payment.tx_hash =
            "0xFEEDFACE00000000000000000000000000000000000000000000000000000001".to_string();
        let mut theirs_payment = create_test_payment(&other_tenant.id, None);
        theirs_payment.tx_hash =
            "0xfeedface00000000000000000000000000000000000000000000000000000002".to_string();
        for p in [&ours_payment, &theirs_payment] {
            PaymentWriter::upsert(&ds, p).await.unwrap();
        }

        // Hash prefix, case-insensitive, ignoring the stored casing.
        let (total, rows) =
            PaymentReader::query(&ds, &PaymentQueryParams::new().with_search("0xFEEDFACE"))
                .await
                .unwrap();
        assert_eq!(total, 2);
        assert_eq!(rows.len(), 2);

        // Symbol and sender are substrings.
        for term in ["eth", "cdef1234"] {
            let (total, _) =
                PaymentReader::query(&ds, &PaymentQueryParams::new().with_search(term))
                    .await
                    .unwrap();
            assert_eq!(total, 2, "substring search on {term:?}");
        }

        // A hash fragment that is not a prefix must not match: the predicate is
        // anchored so it can one day use an index.
        let (total, _) =
            PaymentReader::query(&ds, &PaymentQueryParams::new().with_search("feedface0000"))
                .await
                .unwrap();
        assert_eq!(total, 0, "the tx_hash predicate is anchored, `0x` included");

        // And the scope still wins.
        let (total, rows) = PaymentReader::query(
            &ds,
            &PaymentQueryParams::new()
                .with_store_ids(vec![mine])
                .with_search("0xfeedface"),
        )
        .await
        .unwrap();
        assert_eq!(total, 1, "search must not widen the store scope");
        assert_eq!(rows[0].id, ours_payment.id);
    }
}
