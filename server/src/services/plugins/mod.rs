//! The plugin host: the load-time gate, the host API surface, plugin storage,
//! the static page descriptor and its renderer, and the wasmtime runtime that
//! instantiates, calls, bounds and contains a plugin.
//!
//! `registry` is the manifest-and-version-negotiation slice: given a parsed
//! [`payserver_plugin_api::Manifest`], decide whether this host will register
//! it, before any plugin code runs.
//!
//! `merchant_directory`, `filter`, `invoice_issuer` and `payment_observer`
//! are the four host capabilities: read who the merchants are
//! ([`data_service::MerchantDirectoryReader`], defined in the `data-service`
//! crate rather than here since it is a database read, not a host-state
//! decision, but implemented on [`PluginHostApi`] here so it is reachable the
//! same way the other capabilities are), a filter that can refuse invoice
//! creation, creating an invoice on the instance's own store, and being told
//! after the fact that one of those invoices was paid. None of the four
//! depends on wasmtime; each is written and tested against directly.
//!
//! The asymmetry between `filter` and `payment_observer` is deliberate and
//! load-bearing. A filter may refuse a not-yet-created invoice, because that
//! is a billing decision the merchant can resolve by paying. An observer may
//! refuse nothing, because withholding credit for a payment already sent takes
//! a customer's money over a dispute they are not party to. Neither can drift
//! into the other's shape: one returns a verdict and is never told about a
//! payment, the other is told about payments and returns `()`.
//!
//! `storage` is the storage slice: a schema per plugin, created on install,
//! and a migration runner that runs the plugin's own migrations against it
//! on install and upgrade. Typed reads of core data are not a second surface
//! here - a plugin already reaches them through capability 1
//! (`merchant_directory`) below, via `PluginHostApi`, so an unreachable
//! duplicate next to it would only be a second read surface to maintain.
//! `page` and `pages` are the static page descriptor and the renderer host
//! that serves it. No plugin is registered against the renderer until a
//! runtime can produce one, so every page request 404s until then.
//!
//! `runtime` (instantiate/call/deadline/trap on a single plugin) and `host`
//! (action vs. filter dispatch, disable-on-repeated-failure, admin-visible
//! status) are the wasmtime layer that calls into all of the above.
//!
//! `artifacts` and `boot` are what make any of it reachable from a running
//! server. Every module above this line is in-memory and dies with the
//! process; `artifacts` is the wasm on disk and the digest that says it is
//! still the wasm that was installed, and `boot` reads the install records,
//! verifies each artifact against its digest and registers it with the
//! host - disabling, in the database, anything that fails, so a restart
//! does not walk straight back into the same crash.

mod boot;
mod dispatch;
mod filter;
mod invoice_issuer;
mod merchant_directory;
mod payment_observer;
mod pools;
mod storage;

pub use boot::{
    DEFAULT_CALL_DEADLINE, DEFAULT_MAX_FAILURES, PluginBootReport, load_installed_plugins,
    report_boot,
};
pub use dispatch::{
    FILTER_INVOICE_CREATION, PAYMENT_SETTLED, PluginInvoiceCreationFilter, PluginPaymentObserver,
    invoice_creation_filters, own_store_payment_reporting, payment_observers,
};
pub use filter::{
    FilterVerdict, InvoiceCreationFilter, InvoiceCreationFilterRequest,
    run_invoice_creation_filters,
};
pub use invoice_issuer::{
    HostInvoiceIssuer, InvoiceCreateRequest, InvoiceIssuerError, PluginHostApi, enforce_own_store,
};
pub use payment_observer::{
    OwnStorePayment, OwnStorePaymentObserver, OwnStorePaymentReader, PaymentObserverError,
    is_own_store, notify_own_store_payment,
};
pub use pools::{DEFAULT_MAX_IN_FLIGHT, PluginPoolError, PluginPools};
pub use storage::{
    PluginSchema, PluginStorage, PluginStorageError, generate_role_password, role_name,
};

// The host itself is `payserver-plugin-host`, shared with every other
// payserver. Nothing in it knows about EVM, chains or invoices - it compiles
// a module, instantiates it, calls an export under a deadline, contains a
// trap, verifies an artifact's digest and renders a page descriptor - so
// keeping a second copy here meant two implementations to fix a wasm bug in.
// They had already drifted apart within two days of the extraction.
//
// Re-exported rather than made a direct dependency of every caller: the
// modules below (boot, dispatch, filter, invoice_issuer, storage,
// payment_observer, merchant_directory) are the payserver-shaped half and
// stay here, because every one of them names this server's own database or
// its own money path.
pub use payserver_plugin_host::{
    ArtifactError, FilterOutcome, HOST_MODULE, PageElement, PageError, PageHost, PageRenderer,
    PluginArtifacts, PluginCallError, PluginEngine, PluginHost, PluginHostCalls, PluginHostError,
    PluginInstance, PluginLoadError, PluginRegistry, PluginStatusSnapshot, PluginWasmError, Viewer,
    digest, host_version, page,
};

// Capability 1: no new type here, just `data_service::MerchantDirectoryReader`
// re-exported alongside the other two capabilities' names, and implemented on
// `PluginHostApi` by `merchant_directory` (see that module).
pub use data_service::{MerchantAccount, MerchantDirectoryReader, MerchantStore};
