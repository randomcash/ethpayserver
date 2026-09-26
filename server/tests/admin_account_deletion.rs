#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `DELETE /admin/users/{id}`, `GET /admin/users/{id}/stores` and
//! `DELETE /admin/stores/{id}`, against a real database.
//!
//! These handlers wrap logic that is already covered elsewhere - the
//! financial blockers in
//! `data-service/src/postgres/integration_tests/account_deletion.rs`, the
//! cascade in the same file, `AdminAuth`'s admin-only gate in every other
//! admin route - but nothing else exercises the checks that live only in the
//! handlers themselves: refusing a `server_admin` target outright, refusing a
//! store that fails the synthetic-E2E name/ownership gate, and turning a
//! blocked deletion into the 409 an operator (or an automated sweep) actually
//! sees. An automated sweep against these endpoints is exactly what widens
//! their blast radius if any of those checks silently stops firing.

#[path = "admin_account_deletion/account.rs"]
mod account;
#[path = "admin_account_deletion/self_service.rs"]
mod self_service;
#[path = "admin_account_deletion/store/mod.rs"]
mod store;
#[path = "admin_account_deletion/support.rs"]
mod support;
