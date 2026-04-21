//! Public checkout API endpoints.
//!
//! These endpoints are unauthenticated — any customer with an invoice ID
//! can view payment details. No store internals are exposed.

use axum::{
    Json,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use auth::AuthenticationService;
use data_service::{PaymentOptionReader, PaymentReader};
use types::{InvoiceId, InvoiceReader, InvoiceStatus};

use super::invoices::{PaymentOptionResponse, PaymentResponse};
use super::ws::StatusUpdate;
use crate::state::PgAppState;

/// Public checkout response — only payment-relevant fields.
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckoutResponse {
    /// Invoice ID.
    pub id: String,
    /// Invoice currency (e.g., "USD").
    pub currency: String,
    /// Current status.
    pub status: String,
    /// Requested amount.
    pub amount: String,
    /// Amount received so far.
    pub amount_received: String,
    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Whether the invoice is expired.
    pub is_expired: bool,
    /// Whether the invoice is fully paid.
    pub is_paid: bool,
    /// Payment options (addresses, chains, amounts).
    pub payment_options: Vec<PaymentOptionResponse>,
    /// Payments received.
    pub payments: Vec<PaymentResponse>,
}

/// Get public checkout data for an invoice.
///
/// No authentication required. Returns payment-relevant fields only.
pub async fn get_checkout<A>(
    State(state): State<PgAppState<A>>,
    Path(invoice_id): Path<String>,
) -> Result<Json<CheckoutResponse>, StatusCode>
where
    A: AuthenticationService + 'static,
{
    let id = InvoiceId::from_string(invoice_id);

    let invoice = InvoiceReader::get(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let payments = PaymentReader::get_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let options = PaymentOptionReader::get_for_invoice(&*state.data_service, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let now = chrono::Utc::now();

    Ok(Json(CheckoutResponse {
        id: invoice.id.0,
        currency: invoice.currency,
        status: invoice.status.to_string(),
        amount: invoice.amount.clone(),
        amount_received: invoice.amount_received,
        expires_at: invoice.expires_at,
        is_expired: invoice.status == InvoiceStatus::Expired || invoice.expires_at < now,
        is_paid: invoice.status == InvoiceStatus::Paid,
        payment_options: options.into_iter().map(Into::into).collect(),
        payments: payments.into_iter().map(|p| p.into()).collect(),
    }))
}

/// Query params for public checkout WebSocket.
#[derive(Debug, Deserialize)]
pub struct CheckoutWsQuery {
    /// Invoice ID to subscribe to.
    pub invoice_id: String,
}

/// Public WebSocket handler for checkout status updates.
///
/// No auth required. Only forwards events matching the specified invoice_id.
pub async fn checkout_ws_handler<A>(
    ws: WebSocketUpgrade,
    Query(query): Query<CheckoutWsQuery>,
    State(state): State<PgAppState<A>>,
) -> impl IntoResponse
where
    A: AuthenticationService + 'static,
{
    // Validate that the invoice exists
    let id = InvoiceId::from_string(query.invoice_id.clone());
    match InvoiceReader::get(&*state.data_service, &id).await {
        Ok(Some(_)) => {}
        _ => return StatusCode::NOT_FOUND.into_response(),
    }

    let Some(ws_broadcast) = state.ws_broadcast.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let rx = ws_broadcast.subscribe();

    ws.on_upgrade(move |socket| handle_checkout_socket(socket, rx, query.invoice_id))
        .into_response()
}

/// Handle a public checkout WebSocket connection.
///
/// Only forwards StatusUpdate messages matching the given invoice_id.
async fn handle_checkout_socket(
    socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<StatusUpdate>,
    invoice_id: String,
) {
    let (mut sender, mut receiver) = socket.split();

    // Send connected acknowledgement
    let Ok(connected) = serde_json::to_string(&StatusUpdate::Connected) else {
        return;
    };
    if sender.send(Message::Text(connected.into())).await.is_err() {
        return;
    }

    let inv_id = invoice_id.clone();
    let mut send_task = tokio::spawn(async move {
        while let Ok(update) = rx.recv().await {
            // Filter: only forward events for this invoice
            let matches = match &update {
                StatusUpdate::InvoiceStatus { invoice_id, .. } => invoice_id == &inv_id,
                StatusUpdate::PaymentUpdate { invoice_id, .. } => invoice_id == &inv_id,
                StatusUpdate::Ping => true,
                StatusUpdate::Connected => false,
            };
            if !matches {
                continue;
            }
            let msg = match serde_json::to_string(&update) {
                Ok(json) => json,
                Err(_) => continue,
            };
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_checkout_response_serialization() {
        let resp = CheckoutResponse {
            id: "inv_123".to_string(),
            currency: "USD".to_string(),
            status: "pending".to_string(),
            amount: "100.00".to_string(),
            amount_received: "0".to_string(),
            expires_at: chrono::Utc::now(),
            is_expired: false,
            is_paid: false,
            payment_options: vec![],
            payments: vec![],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "inv_123");
        assert_eq!(json["currency"], "USD");
        assert_eq!(json["is_paid"], false);
    }

    #[test]
    fn test_checkout_ws_query_deserialize() {
        let json = serde_json::json!({ "invoice_id": "inv_abc" });
        let query: CheckoutWsQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.invoice_id, "inv_abc");
    }

    /// Ensures `GET /checkout/ws` routes to the WebSocket handler, not to
    /// `get_checkout` with `invoice_id = "ws"`. Axum prioritises static path
    /// segments over dynamic params, but this test guards against a future
    /// routing regression or a rebase that flips the order.
    #[tokio::test]
    async fn test_ws_route_wins_over_invoice_id_param() {
        use axum::Router;
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        // Build a minimal router mirroring the production mount.
        let app: Router = Router::new()
            .route(
                "/{invoice_id}",
                axum::routing::get(
                    |axum::extract::Path(id): axum::extract::Path<String>| async move {
                        format!("invoice={id}")
                    },
                ),
            )
            .route("/ws", axum::routing::get(|| async { "websocket" }));

        // A plain GET /ws (no upgrade headers) must hit the "websocket" handler,
        // not the `{invoice_id}` handler with id="ws".
        let resp = app
            .oneshot(Request::builder().uri("/ws").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"websocket");
    }
}
