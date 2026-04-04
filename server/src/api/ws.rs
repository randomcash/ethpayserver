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
