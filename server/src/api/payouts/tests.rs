#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Authorization-boundary tests for payouts.
//!
//! Two properties are pinned here, both of which the handler used to take on
//! trust from whoever made the request:
//!
//! 1. A payout is computed only from invoices the store in the path owns. The
//!    invoice ids arrive in the request body and used not to be loaded at all,
//!    so a total could be assembled out of payments belonging to someone else.
//! 2. A payout is read back only by the store that owns it. The payout id
//!    arrives in the path next to a store id that was checked, and the payout
//!    itself was not matched against it.
//!
//! Both refusals are 404, never 403: the difference between "not yours" and "no
//! such row" is itself a fact about another merchant's data.

use super::{payable_total, payout_for_store, reject_claimed_invoices};

use alloy_primitives::U256;
use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::Utc;
use data_service::PayoutClaimReader;
use futures::stream::{self, BoxStream};
use types::{
    AssetType, ChainId, InvoiceData, InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus,
    PaymentData, PaymentQueryParams, PaymentReader, PayoutData, PayoutReader, PayoutStatus,
    RepositoryResult, StoreId,
};
use uuid::Uuid;

fn chain() -> ChainId {
    ChainId::evm(1)
}

fn invoice(id: &str, store_id: StoreId) -> InvoiceData {
    InvoiceData {
        id: InvoiceId::from_string(id.to_string()),
        store_id,
        currency: "USD".to_string(),
        status: InvoiceStatus::Paid,
        amount: "100.00".to_string(),
        amount_received: "100.00".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now(),
        metadata: None,
        customer_email: None,
        extra: None,
    }
}

fn confirmed_payment(invoice_id: &str, amount: &str) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: InvoiceId::from_string(invoice_id.to_string()),
        payment_option_id: None,
        chain_id: chain(),
        asset_type: AssetType::Native,
        amount: amount.to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: format!("0x{:064x}", 1),
        block_number: Some(1),
        detected_at: Utc::now(),
        confirmed_at: Some(Utc::now()),
        from_address: Some("0xpayer".to_string()),
        reorged: false,
        extra: None,
        credited_amount: None,
        rate_used: None,
        rate_applied_at: None,
    }
}

fn payout(store_id: StoreId) -> PayoutData {
    PayoutData {
        id: Uuid::new_v4(),
        store_id,
        invoice_ids: vec!["inv-1".to_string()],
        destination_address: "0xmerchant".to_string(),
        chain_id: chain(),
        asset_type: "native".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        amount: "1000".to_string(),
        tx_hash: None,
        status: PayoutStatus::Pending,
        fee_amount: None,
        error_message: None,
        created_at: Utc::now(),
        confirmed_at: None,
    }
}

/// A store of invoices, their payments, one payout, and a fixed answer to
/// "which of these invoices are already claimed".
#[derive(Default)]
struct StubData {
    invoices: Vec<InvoiceData>,
    payments: Vec<PaymentData>,
    payout: Option<PayoutData>,
    claimed: Vec<String>,
}

#[async_trait]
impl InvoiceReader for StubData {
    async fn get(&self, id: &InvoiceId) -> RepositoryResult<Option<InvoiceData>> {
        Ok(self.invoices.iter().find(|i| i.id == *id).cloned())
    }

    async fn query(&self, _: &InvoiceQueryParams) -> RepositoryResult<(i64, Vec<InvoiceData>)> {
        unimplemented!("not exercised by the payout gate")
    }

    async fn get_expired(&self) -> RepositoryResult<Vec<InvoiceData>> {
        unimplemented!("not exercised by the payout gate")
    }

    fn stream_expired_pending(&self) -> BoxStream<'_, RepositoryResult<InvoiceId>> {
        Box::pin(stream::empty())
    }
}

#[async_trait]
impl PaymentReader for StubData {
    async fn get_valid_for_invoice(
        &self,
        invoice_id: &InvoiceId,
    ) -> RepositoryResult<Vec<PaymentData>> {
        Ok(self
            .payments
            .iter()
            .filter(|p| p.invoice_id == *invoice_id)
            .cloned()
            .collect())
    }

    async fn get(&self, _: Uuid) -> RepositoryResult<Option<PaymentData>> {
        unimplemented!("not exercised by the payout gate")
    }
    async fn get_for_invoice(&self, _: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        unimplemented!("not exercised by the payout gate")
    }
    async fn get_awaiting_confirmation(&self) -> RepositoryResult<Vec<PaymentData>> {
        unimplemented!("not exercised by the payout gate")
    }
    async fn has_valid_payments(&self, _: &InvoiceId) -> RepositoryResult<bool> {
        unimplemented!("not exercised by the payout gate")
    }
    async fn query(&self, _: &PaymentQueryParams) -> RepositoryResult<(i64, Vec<PaymentData>)> {
        unimplemented!("not exercised by the payout gate")
    }
}

#[async_trait]
impl PayoutReader for StubData {
    async fn get_payout(&self, id: Uuid) -> RepositoryResult<Option<PayoutData>> {
        Ok(self.payout.clone().filter(|p| p.id == id))
    }

    async fn get_payouts_for_store(
        &self,
        _: StoreId,
        _: i64,
        _: i64,
    ) -> RepositoryResult<(i64, Vec<PayoutData>)> {
        unimplemented!("not exercised by the payout gate")
    }

    async fn get_active_payouts(&self) -> RepositoryResult<Vec<PayoutData>> {
        unimplemented!("not exercised by the payout gate")
    }
}

#[async_trait]
impl PayoutClaimReader for StubData {
    async fn invoice_ids_already_claimed(
        &self,
        _: StoreId,
        invoice_ids: &[String],
    ) -> RepositoryResult<Vec<String>> {
        Ok(self
            .claimed
            .iter()
            .filter(|id| invoice_ids.contains(id))
            .cloned()
            .collect())
    }
}

// =============================================================================
// Whose invoices a payout may be computed from
// =============================================================================

#[tokio::test]
async fn a_store_is_paid_for_its_own_invoices() {
    // The control for the two tests below: without this, a fix that refused
    // every invoice would look like it worked.
    let mine = StoreId(Uuid::new_v4());
    let data = StubData {
        invoices: vec![invoice("inv-1", mine)],
        payments: vec![confirmed_payment("inv-1", "1000")],
        ..StubData::default()
    };

    let total = payable_total(&data, mine, &["inv-1".to_string()], &chain(), "ETH")
        .await
        .expect("a store's own invoice is payable");

    assert_eq!(total, U256::from(1000));
}

#[tokio::test]
async fn another_stores_invoice_is_never_payable() {
    let mine = StoreId(Uuid::new_v4());
    let theirs = StoreId(Uuid::new_v4());
    let data = StubData {
        invoices: vec![invoice("inv-theirs", theirs)],
        payments: vec![confirmed_payment("inv-theirs", "1000")],
        ..StubData::default()
    };

    let refused = payable_total(&data, mine, &["inv-theirs".to_string()], &chain(), "ETH").await;

    assert_eq!(
        refused,
        Err(StatusCode::NOT_FOUND),
        "an invoice owned by another store must not contribute to this store's payout"
    );
}

#[tokio::test]
async fn a_borrowed_invoice_cannot_be_hidden_among_owned_ones() {
    // Mixing one foreign id into a list of owned ones must refuse the whole
    // request, not silently sum the part it is allowed to see.
    let mine = StoreId(Uuid::new_v4());
    let theirs = StoreId(Uuid::new_v4());
    let data = StubData {
        invoices: vec![invoice("inv-1", mine), invoice("inv-theirs", theirs)],
        payments: vec![
            confirmed_payment("inv-1", "1000"),
            confirmed_payment("inv-theirs", "9000"),
        ],
        ..StubData::default()
    };

    let refused = payable_total(
        &data,
        mine,
        &["inv-1".to_string(), "inv-theirs".to_string()],
        &chain(),
        "ETH",
    )
    .await;

    assert_eq!(refused, Err(StatusCode::NOT_FOUND));
}

#[tokio::test]
async fn an_unknown_invoice_answers_exactly_as_a_foreign_one() {
    // The two must be indistinguishable from outside, or the status code
    // becomes an oracle for which invoice ids exist.
    let mine = StoreId(Uuid::new_v4());
    let theirs = StoreId(Uuid::new_v4());
    let data = StubData {
        invoices: vec![invoice("inv-theirs", theirs)],
        ..StubData::default()
    };

    let unknown = payable_total(&data, mine, &["inv-nope".to_string()], &chain(), "ETH").await;
    let foreign = payable_total(&data, mine, &["inv-theirs".to_string()], &chain(), "ETH").await;

    assert_eq!(unknown, Err(StatusCode::NOT_FOUND));
    assert_eq!(unknown, foreign);
}

// =============================================================================
// Claiming the same money twice
// =============================================================================

#[tokio::test]
async fn unclaimed_invoices_pass_the_duplicate_guard() {
    let mine = StoreId(Uuid::new_v4());
    let data = StubData::default();

    reject_claimed_invoices(&data, mine, &["inv-1".to_string()])
        .await
        .expect("an invoice no payout names may be paid out");
}

#[tokio::test]
async fn an_invoice_already_claimed_by_a_payout_is_refused() {
    let mine = StoreId(Uuid::new_v4());
    let data = StubData {
        claimed: vec!["inv-1".to_string()],
        ..StubData::default()
    };

    let refused =
        reject_claimed_invoices(&data, mine, &["inv-1".to_string(), "inv-2".to_string()]).await;

    assert_eq!(
        refused,
        Err(StatusCode::CONFLICT),
        "money already claimed by a payout must not be claimed a second time"
    );
}

// =============================================================================
// Reading a payout back
// =============================================================================

#[tokio::test]
async fn a_store_reads_back_its_own_payout() {
    let mine = StoreId(Uuid::new_v4());
    let existing = payout(mine);
    let id = existing.id;
    let data = StubData {
        payout: Some(existing),
        ..StubData::default()
    };

    let found = payout_for_store(&data, mine, id)
        .await
        .expect("a store's own payout is readable");

    assert_eq!(found.id, id);
}

#[tokio::test]
async fn another_stores_payout_is_not_readable() {
    let mine = StoreId(Uuid::new_v4());
    let theirs = StoreId(Uuid::new_v4());
    let existing = payout(theirs);
    let id = existing.id;
    let data = StubData {
        payout: Some(existing),
        ..StubData::default()
    };

    let refused = payout_for_store(&data, mine, id).await.err();

    assert_eq!(
        refused,
        Some(StatusCode::NOT_FOUND),
        "a payout's amount and destination address belong to the store that owns it"
    );
}
