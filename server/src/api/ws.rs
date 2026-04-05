//! WebSocket endpoint for real-time invoice and payment status updates.
//!
//! Clients connect to `/ws` and receive JSON-encoded status updates whenever
//! invoice or payment states change. The connection requires authentication
//! via a `token` query parameter (session ID).

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use auth::SessionService;

use crate::state::PgAppState;

/// Query parameters for WebSocket connection.
#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// Session token for authentication.
    pub token: String,
}

/// Status update sent to WebSocket clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StatusUpdate {
    /// Invoice status changed.
    #[serde(rename = "invoice_status")]
    InvoiceStatus { invoice_id: String, status: String },
    /// Payment received or updated.
    #[serde(rename = "payment_update")]
    PaymentUpdate {
        payment_id: String,
        invoice_id: String,
        status: String,
        amount: Option<String>,
    },
    /// Connection acknowledged.
    #[serde(rename = "connected")]
    Connected,
    /// Server-sent ping.
    #[serde(rename = "ping")]
    Ping,
}

/// Shared broadcast channel for status updates.
#[derive(Clone)]
pub struct WsBroadcast {
    tx: broadcast::Sender<StatusUpdate>,
}

impl WsBroadcast {
    /// Create a new broadcast channel with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Send a status update to all connected clients.
    pub fn send(&self, update: StatusUpdate) {
        // Ignore send errors (no receivers).
        let _ = self.tx.send(update);
    }

    /// Subscribe to status updates.
    pub fn subscribe(&self) -> broadcast::Receiver<StatusUpdate> {
        self.tx.subscribe()
    }
}

/// WebSocket upgrade handler.
///
/// Authenticates the user via the `token` query parameter, then upgrades
/// the HTTP connection to a WebSocket.
pub async fn ws_handler<A>(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<PgAppState<A>>,
) -> impl IntoResponse
where
    A: SessionService + 'static,
{
    // Validate session token
    let session_id = match uuid::Uuid::parse_str(&query.token) {
        Ok(uuid) => auth::SessionId(uuid),
        Err(_) => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
    };

    match state.auth_service.validate_session(session_id).await {
        Ok(_) => {}
        Err(_) => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
    }

    // Get broadcast receiver
    let ws_broadcast = state
        .ws_broadcast
        .as_ref()
        .expect("WsBroadcast must be configured");
    let rx = ws_broadcast.subscribe();

    ws.on_upgrade(move |socket| handle_socket(socket, rx))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(socket: WebSocket, mut rx: broadcast::Receiver<StatusUpdate>) {
    let (mut sender, mut receiver) = socket.split();

    // Send connected acknowledgement
    let connected = serde_json::to_string(&StatusUpdate::Connected).unwrap();
    if sender.send(Message::Text(connected.into())).await.is_err() {
        return;
    }

    // Spawn a task to forward broadcast messages to the client
    let mut send_task = tokio::spawn(async move {
        while let Ok(update) = rx.recv().await {
            let msg = match serde_json::to_string(&update) {
                Ok(json) => json,
                Err(_) => continue,
            };
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Spawn a task to handle incoming messages (ping/pong, close)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    // Wait for either task to complete, then abort the other
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_update_serde_invoice_status() {
        let update = StatusUpdate::InvoiceStatus {
            invoice_id: "inv_1".to_string(),
            status: "paid".to_string(),
        };
        let json = serde_json::to_value(&update).unwrap();
        assert_eq!(json["type"], "invoice_status");
        assert_eq!(json["invoice_id"], "inv_1");
        assert_eq!(json["status"], "paid");

        let parsed: StatusUpdate = serde_json::from_value(json).unwrap();
        assert!(
            matches!(parsed, StatusUpdate::InvoiceStatus { invoice_id, status } if invoice_id == "inv_1" && status == "paid")
        );
    }

    #[test]
    fn test_status_update_serde_payment_update() {
        let update = StatusUpdate::PaymentUpdate {
            payment_id: "pay_1".to_string(),
            invoice_id: "inv_1".to_string(),
            status: "confirmed".to_string(),
            amount: Some("1.5".to_string()),
        };
        let json = serde_json::to_value(&update).unwrap();
        assert_eq!(json["type"], "payment_update");
        assert_eq!(json["payment_id"], "pay_1");
        assert_eq!(json["amount"], "1.5");

        // amount = None
        let update_no_amount = StatusUpdate::PaymentUpdate {
            payment_id: "pay_2".to_string(),
            invoice_id: "inv_2".to_string(),
            status: "detecting".to_string(),
            amount: None,
        };
        let json2 = serde_json::to_value(&update_no_amount).unwrap();
        assert!(json2.get("amount").unwrap().is_null());
    }

    #[test]
    fn test_status_update_serde_connected_and_ping() {
        let connected_json = serde_json::to_string(&StatusUpdate::Connected).unwrap();
        assert_eq!(connected_json, r#"{"type":"connected"}"#);

        let ping_json = serde_json::to_string(&StatusUpdate::Ping).unwrap();
        assert_eq!(ping_json, r#"{"type":"ping"}"#);

        let parsed: StatusUpdate = serde_json::from_str(&connected_json).unwrap();
        assert!(matches!(parsed, StatusUpdate::Connected));
    }

    #[test]
    fn test_ws_query_deserialize() {
        let json = serde_json::json!({ "token": "abc-123" });
        let query: WsQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.token, "abc-123");
    }

    #[test]
    fn test_ws_broadcast_send_no_receivers() {
        let broadcast = WsBroadcast::new(16);
        // Should not panic even with no receivers
        broadcast.send(StatusUpdate::Ping);
    }

    #[tokio::test]
    async fn test_ws_broadcast_send_receive() {
        let broadcast = WsBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        broadcast.send(StatusUpdate::Connected);
        broadcast.send(StatusUpdate::InvoiceStatus {
            invoice_id: "inv_1".to_string(),
            status: "paid".to_string(),
        });

        let msg1 = rx.recv().await.unwrap();
        assert!(matches!(msg1, StatusUpdate::Connected));

        let msg2 = rx.recv().await.unwrap();
        assert!(
            matches!(msg2, StatusUpdate::InvoiceStatus { invoice_id, .. } if invoice_id == "inv_1")
        );
    }

    #[tokio::test]
    async fn test_ws_broadcast_multiple_subscribers() {
        let broadcast = WsBroadcast::new(16);
        let mut rx1 = broadcast.subscribe();
        let mut rx2 = broadcast.subscribe();

        broadcast.send(StatusUpdate::Ping);

        assert!(matches!(rx1.recv().await.unwrap(), StatusUpdate::Ping));
        assert!(matches!(rx2.recv().await.unwrap(), StatusUpdate::Ping));
    }
}
