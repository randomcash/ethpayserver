//! In-memory `InvoiceCreationWriter` for `InMemoryDataService`.

use async_trait::async_trait;
use types::{
    InvoiceData, InvoiceWriter, PaymentOptionData, PaymentOptionWriter, RepositoryResult,
    WatchedAddressWriter,
};

use super::InMemoryDataService;
use crate::InvoiceCreationWriter;

/// Not transactional, unlike Postgres: it performs the same three writes in
/// order, so a failure partway through is not rolled back here.
#[async_trait]
impl InvoiceCreationWriter for InMemoryDataService {
    async fn create_invoice_with_options(
        &self,
        invoice: &InvoiceData,
        options: &[PaymentOptionData],
    ) -> RepositoryResult<()> {
        InvoiceWriter::upsert(self, invoice).await?;
        for option in options {
            PaymentOptionWriter::create(self, option).await?;
            WatchedAddressWriter::upsert(
                self,
                &option.payment_address,
                &option.id,
                &option.chain_id,
                option.token_address.as_deref(),
            )
            .await?;
        }
        Ok(())
    }
}
