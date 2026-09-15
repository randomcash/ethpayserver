//! Capability 1 (RCS-300), reachable from the host API surface.
//!
//! The reader itself is `data_service::MerchantDirectoryReader` (see that
//! module's doc for why it lives there rather than here or in `auth`) - what
//! this adds is the missing other half: unlike capabilities 2 and 3, capability
//! 1 had no method on `PluginHostApi` and no re-export alongside
//! `HostInvoiceIssuer`/`InvoiceCreationFilter`, so nothing under
//! `services::plugins` could actually reach it. This is a direct delegation,
//! not a new implementation: `PluginHostApi` already holds the same
//! `PgDataService` that `HostInvoiceIssuer` reads and writes through, so
//! `list_accounts`/`list_stores` here are exactly
//! `data_service::MerchantDirectoryReader`, already tested against a real
//! database by `data-service`'s own integration tests - there is no new logic
//! to break.

use async_trait::async_trait;
use auth::SessionService;
use data_service::{MerchantAccount, MerchantDirectoryReader, MerchantStore};
use types::RepositoryResult;

use super::PluginHostApi;

#[async_trait]
impl<A: SessionService + 'static> MerchantDirectoryReader for PluginHostApi<A> {
    async fn list_accounts(
        &self,
        offset: i64,
        limit: i64,
    ) -> RepositoryResult<Vec<MerchantAccount>> {
        self.data_service().list_accounts(offset, limit).await
    }

    async fn list_stores(&self, offset: i64, limit: i64) -> RepositoryResult<Vec<MerchantStore>> {
        self.data_service().list_stores(offset, limit).await
    }
}
