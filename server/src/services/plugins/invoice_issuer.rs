//! Capability 3: create an invoice, on the instance's own store
//! only.
//!
//! SENSITIVE: this is a write to the money path, granted to a plugin. The
//! instance sells subscriptions to itself, so the billing plugin has to
//! *issue* the invoice the merchant pays - an ordinary payment, to an
//! ordinary invoice, on our store, settled to our xpub. The non-custodial
//! guarantee a merchant relies on is unchanged either way: this never derives
//! from, or writes to, a merchant's own wallet.
//!
//! The host enforces which store, not the plugin: [`enforce_own_store`] is
//! the entire safety property this module exists for. A plugin that could
//! name any store could invoice a merchant's customers, under the merchant's
//! name, to an address the merchant never chose - so `request.store_id` is
//! checked against the host's own configured store and never substituted or
//! trusted further.
//!
//! No FX: the amount is taken in the asset the instance already accepts, so
//! creating a subscription invoice needs no rate provider. That is a
//! deliberate narrowing of what the HTTP `create_invoice` endpoint offers a
//! merchant, not an oversight - the instance controls its own payment
//! methods, so it can always price its own subscription in one of them.
//!
//! Address derivation itself is not reimplemented here: `derive_payment_options`
//! is the same function the HTTP endpoint calls, reused via a crate-visible
//! re-export from `api::invoices` (see the comment there), so a plugin-issued
//! invoice's address comes from the exact same wallet-resolution and
//! derivation-counter code the rest of the server already relies on.

use chrono::Utc;

use ::types::currency::DEFAULT_INVOICE_EXPIRATION_SECS;
use ::types::{InvoiceId, StoreId, traits::InvoiceData};
use async_trait::async_trait;
use auth::SessionService;

use crate::api::invoices;
use crate::state::PgAppState;

/// Why capability 3 refused a request.
#[derive(Debug, thiserror::Error)]
pub enum InvoiceIssuerError {
    /// The request named a store other than the one this host issues its
    /// own invoices from. This is the refusal ticket test 1 exists to pin:
    /// remove the check in [`enforce_own_store`] and a plugin could name any
    /// merchant's store instead.
    #[error(
        "plugin requested invoice creation on store {requested}, which is not this host's own store"
    )]
    ForbiddenStore { requested: StoreId },

    #[error("store {store_id} has no enabled payment method for asset {asset_symbol}")]
    UnsupportedAsset {
        store_id: StoreId,
        asset_symbol: String,
    },

    #[error("store {0} has no wallet to receive payments")]
    NoWallet(StoreId),

    #[error("invalid amount: {0}")]
    InvalidAmount(String),

    #[error("failed to create invoice: {0}")]
    Internal(String),
}

/// What a plugin asks for. `asset_symbol` must match one of the store's own
/// enabled payment methods exactly - see the module doc on why there is no
/// currency-conversion path here.
#[derive(Debug, Clone)]
pub struct InvoiceCreateRequest {
    pub store_id: StoreId,
    pub asset_symbol: String,
    pub amount: String,
    pub metadata: Option<serde_json::Value>,
    pub customer_email: Option<String>,
}

/// The entire safety property capability 3 exists to provide: `requested`
/// must be the host's own store, or the request is refused before anything
/// is read or written.
///
/// A version of [`PluginHostApi::invoice_create`] that let the plugin's
/// requested store through unchecked would pass every other test for this
/// capability - only a test that calls this function (or exercises it via
/// `invoice_create`) with a foreign `store_id` catches it.
pub fn enforce_own_store(
    own_store_id: StoreId,
    requested: StoreId,
) -> Result<(), InvoiceIssuerError> {
    if requested != own_store_id {
        return Err(InvoiceIssuerError::ForbiddenStore { requested });
    }
    Ok(())
}

/// Create an invoice, on the instance's own store only.
#[async_trait]
pub trait HostInvoiceIssuer: Send + Sync {
    /// The one store this issuer will issue on.
    ///
    /// On the trait rather than only on the concrete type because the wasm
    /// host call needs it: a plugin's request carries no store, so the host
    /// has to supply one, and the only correct one is the issuer's own. A
    /// caller that had to be *told* the store could be told the wrong one,
    /// which is precisely the hole `enforce_own_store` exists to close -
    /// so it is read from the thing that will enforce it.
    fn own_store_id(&self) -> StoreId;

    async fn invoice_create(
        &self,
        request: InvoiceCreateRequest,
    ) -> Result<InvoiceData, InvoiceIssuerError>;
}

/// The host side of the plugin API's write capability, bound to one
/// instance's own store.
///
/// Wired to the plugin runtime: `host_calls::PluginCalls::invoice_create`
/// reaches this through a [`super::host_calls::DeferredIssuer`] that
/// `server.rs` publishes at boot once a billing store is configured, so a
/// wasm plugin's `invoice_create` import lands here, not on a stub.
///
/// That import binding itself - a compiled wasm guest actually reaching
/// `PluginCalls::invoice_create` through wasmtime, not just a Rust-level call
/// to it - is not code in this repository, so it cannot be shown in this
/// diff: the linker that binds the `invoice_create` import is
/// `host_linker` in `payserver-plugin-host::runtime` (the crate this
/// workspace pins by `rev` in the root `Cargo.toml`), and the guest-side
/// round trip is exercised end to end, through a real compiled wasm module
/// and a real `wasmtime::Linker`, by
/// `a_plugin_can_ask_the_host_to_issue_an_invoice` in that crate's
/// `runtime.rs` tests.
///
/// This type's own half - that a published `PluginHostApi`, not a test
/// double, actually creates a real, correctly-priced invoice against a real
/// database - is `a_real_issuer_creates_a_real_payable_invoice_in_base_units`
/// in `server/tests/plugin_invoice_issuer.rs`.
pub struct PluginHostApi<A> {
    state: PgAppState<A>,
    own_store_id: StoreId,
}

impl<A> PluginHostApi<A> {
    pub fn new(state: PgAppState<A>, own_store_id: StoreId) -> Self {
        Self {
            state,
            own_store_id,
        }
    }

    /// Shared with sibling capability modules under `services::plugins` (see
    /// `merchant_directory.rs`) that read through the same data service this
    /// one writes through, rather than a second connection of their own.
    pub(super) fn data_service(&self) -> &data_service::PgDataService {
        &self.state.data_service
    }
}

#[async_trait]
impl<A: SessionService + 'static> HostInvoiceIssuer for PluginHostApi<A> {
    /// Shared with `payment_observer.rs` so the reconciliation read is bound
    /// to exactly the store `enforce_own_store` bounds the write to. Two
    /// different notions of "our store" between the two would be a hole.
    fn own_store_id(&self) -> StoreId {
        self.own_store_id
    }

    async fn invoice_create(
        &self,
        request: InvoiceCreateRequest,
    ) -> Result<InvoiceData, InvoiceIssuerError> {
        enforce_own_store(self.own_store_id, request.store_id)?;

        let mut payment_methods =
            data_service::StorePaymentMethodReader::get_enabled_payment_methods(
                &*self.state.data_service,
                request.store_id.0,
            )
            .await
            .map_err(|e| InvoiceIssuerError::Internal(e.to_string()))?;

        if let Some(policy) = data_service::StoreTokenPolicyReader::get_token_policy(
            &*self.state.data_service,
            request.store_id.0,
        )
        .await
        .map_err(|e| InvoiceIssuerError::Internal(e.to_string()))?
        {
            invoices::apply_token_policy_filter(&mut payment_methods, &policy);
        }

        let method_idx = payment_methods
            .iter()
            .position(|pm| pm.asset_symbol.eq_ignore_ascii_case(&request.asset_symbol))
            .ok_or_else(|| InvoiceIssuerError::UnsupportedAsset {
                store_id: request.store_id,
                asset_symbol: request.asset_symbol.clone(),
            })?;

        if payment_methods[method_idx].wallet_id.is_none() {
            return Err(InvoiceIssuerError::NoWallet(request.store_id));
        }

        let crypto_amount = invoices::convert_human_to_smallest_unit(
            &request.amount,
            payment_methods[method_idx].decimals,
        )
        .map_err(|e| InvoiceIssuerError::InvalidAmount(e.to_string()))?;

        let expires_at =
            Utc::now() + chrono::Duration::seconds(DEFAULT_INVOICE_EXPIRATION_SECS as i64);

        let invoice = InvoiceData {
            id: InvoiceId::new(),
            store_id: request.store_id,
            currency: payment_methods[method_idx].asset_symbol.clone(),
            status: ::types::InvoiceStatus::Pending,
            amount: request.amount,
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at,
            metadata: request.metadata,
            customer_email: request.customer_email,
            extra: None,
        };

        let derived = invoices::derive_payment_options(
            &self.state,
            &invoice,
            &payment_methods,
            vec![(method_idx, crypto_amount, None, None)],
        )
        .await
        .map_err(|(_, json)| InvoiceIssuerError::Internal(json.0.to_string()))?;

        let options: Vec<_> = derived.iter().map(|d| d.option.clone()).collect();
        data_service::InvoiceCreationWriter::create_invoice_with_options(
            &*self.state.data_service,
            &invoice,
            &options,
        )
        .await
        .map_err(|e| InvoiceIssuerError::Internal(e.to_string()))?;

        for d in &derived {
            invoices::notify_evm_watch(
                &self.state,
                &invoice.id.0,
                &d.option.payment_address,
                &d.option.chain_id,
                d.option.token_address.as_deref(),
                d.address,
                d.expected_amount,
                d.token_contract,
            )
            .await;
        }

        Ok(invoice)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::Arc;

    use auth::{Result as AuthResult, Session, SessionId, UserInfo};

    use super::*;
    use crate::state::PgAppState;

    /// Ticket test 1: a plugin calling invoice-create for a merchant's store
    /// id is refused. This is the pure enforcement decision, isolated from
    /// the database - `invoice_create` calls exactly this before it touches
    /// anything.
    #[test]
    fn forbids_a_merchants_store() {
        let own_store = StoreId::new();
        let merchants_store = StoreId::new();

        let err = enforce_own_store(own_store, merchants_store).unwrap_err();
        assert!(matches!(
            err,
            InvoiceIssuerError::ForbiddenStore { requested } if requested == merchants_store
        ));
    }

    #[test]
    fn allows_the_hosts_own_store() {
        let own_store = StoreId::new();
        assert!(enforce_own_store(own_store, own_store).is_ok());
    }

    /// Never actually called: `invoice_create` refuses a foreign store before
    /// it touches session state, and this test never reaches the host's own
    /// store either.
    struct UnusedSessionService;

    #[async_trait]
    impl SessionService for UnusedSessionService {
        async fn validate_session(
            &self,
            _session_id: SessionId,
        ) -> AuthResult<(UserInfo, Session)> {
            unimplemented!("not exercised by invoice_create's store check")
        }
        async fn logout(&self, _session_id: SessionId) -> AuthResult<()> {
            unimplemented!("not exercised by invoice_create's store check")
        }
        async fn logout_all(&self, _session_id: SessionId) -> AuthResult<()> {
            unimplemented!("not exercised by invoice_create's store check")
        }
        async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
            unimplemented!("not exercised by invoice_create's store check")
        }
    }

    /// A pool that never connects: `enforce_own_store` is the first thing
    /// `invoice_create` does, and it returns before the pool is ever touched.
    /// `connect_lazy` (already used the same way in
    /// `api::api_key_rate_limit`'s tests) defers the actual TCP connection to
    /// first query, so this test needs no live database.
    fn host_api(own_store: StoreId) -> PluginHostApi<UnusedSessionService> {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/unused").unwrap();
        let state = PgAppState::new(
            Arc::new(data_service::PgDataService::new(pool)),
            Arc::new(UnusedSessionService),
            None,
            Arc::new(rates::NoOpRateProvider),
            Arc::new(crate::services::email::NoopEmailSender),
        );
        PluginHostApi::new(state, own_store)
    }

    /// Review finding, fixed: the earlier tests here only exercised
    /// the isolated `enforce_own_store` helper. Nothing called
    /// `HostInvoiceIssuer::invoice_create` itself - the method a plugin
    /// actually invokes - so a regression that dropped the `?`, ignored the
    /// result, or reordered the check after a DB read would have left every
    /// test in this file green. This calls the real trait method with a
    /// merchant's store id and checks the refusal comes from it.
    #[tokio::test]
    async fn invoice_create_refuses_a_merchants_store() {
        let own_store = StoreId::new();
        let merchants_store = StoreId::new();
        let api = host_api(own_store);

        let request = InvoiceCreateRequest {
            store_id: merchants_store,
            asset_symbol: "USDC".to_string(),
            amount: "10.00".to_string(),
            metadata: None,
            customer_email: None,
        };

        let err = api.invoice_create(request).await.unwrap_err();
        assert!(matches!(
            err,
            InvoiceIssuerError::ForbiddenStore { requested } if requested == merchants_store
        ));
    }
}
