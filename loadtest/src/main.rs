//! ethpayserver load-testing suite using Goose.
//!
//! Implements four scenarios from the sizing baseline spec:
//!   1. invoice-create     — POST /invoices at sustained rate
//!   2. invoice-list       — GET /invoices with concurrent readers
//!   3. webhook-burst      — POST /invoices with webhook_url attached
//!   4. websocket-connect  — see the separate `loadtest-ws` binary
//!
//! # Environment variables
//!
//! | Variable                       | Required | Description                              |
//! |--------------------------------|----------|------------------------------------------|
//! | `LOADTEST_API_KEY`             | yes      | API key (`ak_…`) for authentication      |
//! | `LOADTEST_STORE_ID`            | yes      | UUID of the store to test against        |
//! | `LOADTEST_WEBHOOK_RECEIVER_URL`| no       | Webhook receiver (default localhost:9999)|
//!
//! # Usage
//!
//! ```sh
//! # Run all HTTP scenarios against localhost:
//! LOADTEST_API_KEY=ak_test_xxx LOADTEST_STORE_ID=<uuid> \
//!   cargo run -p loadtest --bin loadtest -- \
//!     --host http://localhost:3000 --users 20 --run-time 60s
//!
//! # Run only the invoice-create scenario:
//! LOADTEST_API_KEY=ak_test_xxx LOADTEST_STORE_ID=<uuid> \
//!   cargo run -p loadtest --bin loadtest -- \
//!     --host http://localhost:3000 --scenarios "InvoiceCreate"
//! ```

mod config;
mod scenarios;
mod session;

use goose::prelude::*;

use crate::config::Config;
use crate::scenarios::invoice_create::create_invoice;
use crate::scenarios::invoice_list::list_invoices;
use crate::scenarios::webhook_burst::create_invoice_with_webhook;
use crate::session::Session;

/// Setup transaction: loads config from env and stores it as session data
/// so every subsequent transaction has access to auth credentials.
async fn setup(user: &mut GooseUser) -> TransactionResult {
    let config = Config::from_env();
    user.set_session_data(Session {
        api_key: config.api_key,
        store_id: config.store_id,
        webhook_receiver_url: config.webhook_receiver_url,
    });
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("InvoiceCreate")
                .register_transaction(transaction!(setup).set_on_start())
                .register_transaction(transaction!(create_invoice).set_name("POST /invoices")),
        )
        .register_scenario(
            scenario!("InvoiceList")
                .register_transaction(transaction!(setup).set_on_start())
                .register_transaction(transaction!(list_invoices).set_name("GET /invoices")),
        )
        .register_scenario(
            scenario!("WebhookBurst")
                .register_transaction(transaction!(setup).set_on_start())
                .register_transaction(
                    transaction!(create_invoice_with_webhook).set_name("POST /invoices (webhook)"),
                ),
        )
        .execute()
        .await?;

    Ok(())
}
