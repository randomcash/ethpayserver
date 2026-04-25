//! Invoice creation scenario — write-heavy load test.
//!
//! Target: 50 RPS sustained, p95 latency < 200ms on a 2-vCPU / 4 GiB box.

use goose::prelude::*;

use crate::session::Session;

/// Goose transaction: POST /invoices with a random amount.
pub async fn create_invoice(user: &mut GooseUser) -> TransactionResult {
    let session = user.get_session_data::<Session>().unwrap().clone();

    let amount = format!("{:.2}", 1.0 + rand::random::<f64>() * 999.0);
    let body = serde_json::json!({
        "store_id": session.store_id,
        "currency": "USD",
        "amount": amount,
        "expiration_seconds": 900,
    });

    let builder = user
        .get_request_builder(&GooseMethod::Post, "/invoices")?
        .header("Authorization", format!("Bearer {}", session.api_key))
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).unwrap());

    let goose_request = GooseRequest::builder()
        .set_request_builder(builder)
        .expect_status_code(201)
        .name("POST /invoices")
        .build();

    user.request(goose_request).await?;
    Ok(())
}
