use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::PluginCalls;

/// The issuer a plugin's `invoice_create` reaches, published once the server
/// has one.
///
/// This exists to break a genuine cycle rather than to be clever about
/// initialisation order. An issuer is built around `AppState`; `AppState` is
/// built with the capability implementations that come out of plugin
/// loading; plugin loading is where a plugin is handed its host calls. One
/// of the three has to be late, and this is the one where late is harmless:
/// nothing can call a plugin until the router is serving, and the cell is
/// filled before it does.
///
/// A `OnceLock` rather than a `Mutex` because the write happens once during
/// boot and every read after it is on the money path. It also means the
/// capability cannot be swapped out from under a running plugin - whoever
/// could do that could redirect where invoices are issued.
///
/// Unpublished reads as "this host does not issue invoices", which is the
/// same answer an instance with no billing store gives, and the right one:
/// in both cases there is no store this host would be willing to issue on.
#[derive(Clone, Default)]
pub struct DeferredIssuer(
    Arc<std::sync::OnceLock<Arc<dyn crate::services::plugins::HostInvoiceIssuer>>>,
);

impl DeferredIssuer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the issuer. Returns whether this call is the one that set it;
    /// a second call changes nothing and says so rather than silently
    /// winning or silently losing.
    pub fn publish(&self, issuer: Arc<dyn crate::services::plugins::HostInvoiceIssuer>) -> bool {
        self.0.set(issuer).is_ok()
    }

    fn get(&self) -> Option<&Arc<dyn crate::services::plugins::HostInvoiceIssuer>> {
        self.0.get()
    }
}

impl std::fmt::Debug for DeferredIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeferredIssuer")
            .field(&self.get().is_some())
            .finish()
    }
}

/// What a plugin sends to ask for an invoice.
///
/// Note what is not here: a store. Not an optional one, not one that gets
/// checked - the field does not exist, so a plugin cannot express the
/// request that would have to be refused. `enforce_own_store` still guards
/// the host-side API for ordinary callers; on this path the property holds
/// because there is nothing to enforce it against.
///
/// Every value is a string, for the same reason it is on the storage side:
/// an amount is `NUMERIC(38,18)` and a JSON number is a double.
#[derive(Debug, Deserialize)]
struct InvoiceRequest {
    /// Must match one of the store's own enabled payment methods. There is
    /// no conversion path here - the instance prices its own subscription in
    /// something it already accepts.
    asset_symbol: String,
    amount: String,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    customer_email: Option<String>,
}

/// What the plugin gets back: enough to record the invoice against a
/// subscription and to send the merchant to it.
#[derive(Debug, Serialize)]
struct InvoiceIssued {
    invoice_id: String,
    currency: String,
    amount: String,
    status: String,
    expires_at: String,
    /// Where a merchant pays it. A path rather than a URL: the host does not
    /// reliably know its own external origin, and a plugin that rendered a
    /// wrong absolute URL would send a paying merchant somewhere that is not
    /// this instance.
    checkout_path: String,
}

impl PluginCalls {
    pub(super) fn invoice_create_impl(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let parsed: InvoiceRequest = serde_json::from_slice(request)
            .map_err(|e| format!("could not read the invoice request: {e}"))?;

        let issued = self.issue(parsed)?;
        serde_json::to_vec(&issued)
            .map_err(|e| format!("could not serialise the invoice answer: {e}"))
    }

    fn issue(&self, request: InvoiceRequest) -> Result<InvoiceIssued, String> {
        let Some(issuer) = self.issuer.get().cloned() else {
            return Err(
                "this host does not issue invoices; no own store is configured for it".to_string(),
            );
        };

        // The store is the host's, taken from the issuer that was built
        // around it. `own_store_id()` is the same value `enforce_own_store`
        // would compare against, so the check it performs is trivially
        // satisfied rather than skipped.
        let store_id = issuer.own_store_id();

        let invoice = self
            .handle
            .block_on(
                issuer.invoice_create(crate::services::plugins::InvoiceCreateRequest {
                    store_id,
                    asset_symbol: request.asset_symbol,
                    amount: request.amount,
                    metadata: request.metadata,
                    customer_email: request.customer_email,
                }),
            )
            .map_err(|e| e.to_string())?;

        Ok(InvoiceIssued {
            checkout_path: format!("/checkout/{}", invoice.id.0),
            invoice_id: invoice.id.0.to_string(),
            currency: invoice.currency,
            amount: invoice.amount,
            status: format!("{:?}", invoice.status).to_lowercase(),
            expires_at: invoice.expires_at.to_rfc3339(),
        })
    }
}
