//! Standalone entry point for the EVM PayServer client.
//!
//! This allows the frontend to run independently for development
//! or as a standalone deployment.
//!
//! Note that this bin target is NOT what trunk builds — `index.html` points at
//! the cdylib (`data-target-name="ethpayserver_client"`), so the deployed bundle
//! enters through `ethpayserver_client::init`. Both paths call `mount_app` so the
//! two entry points cannot drift apart.

fn main() {
    ethpayserver_client::mount_app();
}
