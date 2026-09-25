#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Two merchants, A and B, each with their own store, invoice, payment,
//! wallet and API key. Every store-scoped read endpoint this file enumerates
//! (invoices and payments including list, by id and CSV export; stores
//! including by id, list, members, webhook config and token policy; wallets;
//! dashboard aggregates; payouts, refunds and webhook deliveries) is asked
//! for the other tenant's data, by id, by store filter, by the "all stores"
//! path, and authenticated with an API key instead of a session where that
//! axis applies, and must refuse. This is not a claim that literally every
//! handler under `server/src/api` is covered; it is the enumeration of the
//! store/tenant-scoped ones, grown each time a gap was found.
//!
//! This has shipped broken in both directions before: a nil-UUID `store_id`
//! that meant "every store" leaked one merchant's invoices and payments to
//! any authenticated caller, and - separately - the fix for that leak made
//! the "all stores" view admin-only, so a merchant asking for their own
//! stores with no filter got refused instead of an empty-looking answer.
//! Both are the same missing test: nobody asked "can A see B's row", in
//! either the leaking direction or the withholding one.
//!
//! A third shape is a key that carries more than its owner's scope rather
//! than a session that does: the stored row has no scope field narrower than
//! "everything its owner can do", so this schema can produce that in two
//! ways - a bearer-token path that resolves to the wrong owner or skips the
//! per-request tenant check a session goes through (the `api_keys` module's
//! `cross_tenant_reads` and `scope_parity` tests re-run every
//! session-tenancy assertion through the real key-hash lookup for exactly
//! that reason), and a key the active/expiry check should have refused
//! outright still authenticating, carrying its owner's full access past a
//! point it should never have reached (`api_keys::lifecycle`, asserting the
//! rejection itself rather than what a successful call can reach). See that
//! module's doc comment for the full argument.
//!
//! Calls handler functions directly against a real database, the same way
//! `plugin_invoice_creation_filter.rs` does: `AuthenticatedUser` and `State`
//! are plain data the extractors produce, and `server/src/api/**` handlers
//! are pinned to a concrete `State<PgAppState<A>>`, not a generic trait
//! object, so there is no way to run them against `InMemoryDataService`.
//!
//! Every `#[ignore]`'d test below needs `DATABASE_URL` and is not run by the
//! plain `cargo nextest run --workspace` pass. That is not a gap: CI's
//! "Integration tests" step already runs `cargo nextest run -p data-service
//! -p server --run-ignored only` against a real Postgres instance and gates
//! merges on it, the same lane `plugin_invoice_creation_filter.rs` relies on.
//! A test added under this module is exercised by that existing job with no
//! further wiring.
//!
//! Split across files by the resource each covers rather than kept as one
//! file, once the file crossed the repo's line-count ratchet - `support.rs`
//! holds the shared fixtures every other module draws on.

#[path = "cross_tenant_isolation/support.rs"]
mod support;

#[path = "cross_tenant_isolation/api_keys/mod.rs"]
mod api_keys;
#[path = "cross_tenant_isolation/csv_export.rs"]
mod csv_export;
#[path = "cross_tenant_isolation/dashboard.rs"]
mod dashboard;
#[path = "cross_tenant_isolation/invoices.rs"]
mod invoices;
#[path = "cross_tenant_isolation/payments.rs"]
mod payments;
#[path = "cross_tenant_isolation/payouts_refunds_deliveries.rs"]
mod payouts_refunds_deliveries;
#[path = "cross_tenant_isolation/plugin_pages.rs"]
mod plugin_pages;
#[path = "cross_tenant_isolation/store_settings.rs"]
mod store_settings;
#[path = "cross_tenant_isolation/stores.rs"]
mod stores;
#[path = "cross_tenant_isolation/wallets.rs"]
mod wallets;
