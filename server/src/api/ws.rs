//! WebSocket endpoint for real-time invoice and payment status updates.
//!
//! Clients connect to `/ws` and authenticate by sending an auth message as the
//! first frame: `{"type":"auth","token":"SESSION_ID"}`. The server validates
//! the session and then forwards JSON-encoded status updates.
//!
//! Every update is tied to the store it happened in, and a socket receives it
//! only if its user is a server admin or a member of that store. The decision
//! is made per event, against the database, at the moment the event is
//! delivered - not once at connect time - so a member removed while connected
//! stops receiving at once. Anything that prevents the decision (a database
//! error, a user that no longer exists) withholds the event.

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use auth::{SessionService, UserRepository, repository::UserStoreRepository};
use types::{InvoiceId, InvoiceReader, StoreId};

use super::invoices::{StoreScope, verify_store_access_for_query};
use crate::state::PgAppState;

/// Client-to-server messages.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    /// Authentication message — must be the first frame after connection.
    #[serde(rename = "auth")]
    Auth { token: String },
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

/// A status update together with the store it happened in.
///
/// The store id is not part of the wire format: it is what the socket handlers
/// decide delivery on, and is dropped before the update is serialised.
#[derive(Debug, Clone)]
pub struct WsEvent {
    /// Store the invoice or payment belongs to.
    pub store_id: StoreId,
    /// What clients are told.
    pub update: StatusUpdate,
}

/// Shared broadcast channel for status updates.
#[derive(Clone)]
pub struct WsBroadcast {
    tx: broadcast::Sender<WsEvent>,
}

impl WsBroadcast {
    /// Create a new broadcast channel with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish a status update for `store_id`.
    ///
    /// The store is required, not optional: an update with no store would have
    /// no one it could safely be shown to.
    pub fn send(&self, store_id: StoreId, update: StatusUpdate) {
        // Ignore send errors (no receivers).
        let _ = self.tx.send(WsEvent { store_id, update });
    }

    /// Publish a status update for an invoice whose store the caller does not
    /// already hold, looking the store up from the invoice.
    ///
    /// If the store cannot be determined - the lookup fails, or the invoice is
    /// gone - nothing is published: an update that cannot be attributed to a
    /// store cannot be shown to anyone.
    pub async fn send_for_invoice<R>(
        &self,
        reader: &R,
        invoice_id: &InvoiceId,
        update: StatusUpdate,
    ) where
        R: InvoiceReader + ?Sized,
    {
        match reader.get(invoice_id).await {
            Ok(Some(invoice)) => self.send(invoice.store_id, update),
            Ok(None) => tracing::warn!(
                invoice_id = %invoice_id.as_str(),
                "status update dropped: invoice not found, so its store is unknown"
            ),
            Err(e) => tracing::warn!(
                invoice_id = %invoice_id.as_str(),
                error = %e,
                "status update dropped: invoice lookup failed, so its store is unknown"
            ),
        }
    }

    /// Subscribe to status updates.
    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.tx.subscribe()
    }
}

/// Answers, per event, whether one authenticated socket may see a store.
type StoreGate = std::sync::Arc<dyn Fn(StoreId) -> BoxFuture<'static, bool> + Send + Sync>;

/// Whether `user_id` may see events of `store_id`: a server admin, or a member.
///
/// The user is read afresh, so a demotion takes effect on the next event, and
/// the membership check is the one the REST list endpoints use. Fails closed:
/// every error, and a user row that is gone, answers `false`.
pub(crate) async fn may_see_store<D>(
    data_service: &D,
    user_id: auth::UserId,
    store_id: StoreId,
) -> bool
where
    D: UserRepository + UserStoreRepository + ?Sized,
{
    let user = match data_service.get_user(user_id).await {
        Ok(Some(user)) => auth::UserInfo::from(&user),
        Ok(None) | Err(_) => return false,
    };
    match verify_store_access_for_query(data_service, &user, Some(store_id.0)).await {
        Ok(StoreScope::One(_)) => true,
        Ok(_) | Err(_) => false,
    }
}

/// WebSocket upgrade handler.
///
/// Accepts the upgrade immediately. Authentication happens inside the
/// WebSocket session: the client must send `{"type":"auth","token":"..."}` as
/// the first frame. This avoids leaking the session token in URL query
/// parameters (browser history, proxy logs, server access logs).
pub async fn ws_handler<A>(
    ws: WebSocketUpgrade,
    State(state): State<PgAppState<A>>,
) -> impl IntoResponse
where
    A: SessionService + 'static,
{
    // Get broadcast receiver. In deployments without WS configured, return 503
    // rather than panicking the handler task.
    let Some(ws_broadcast) = state.ws_broadcast.as_ref() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let rx = ws_broadcast.subscribe();
    let auth_service = state.auth_service.clone();
    let data_service = state.data_service.clone();

    ws.on_upgrade(move |socket| handle_socket(socket, rx, auth_service, data_service))
        .into_response()
}

/// Maximum time to wait for the auth message before closing the connection.
const AUTH_TIMEOUT_SECS: u64 = 10;

/// Handle an individual WebSocket connection.
///
/// Waits for the client to send an auth message, validates the session,
/// then forwards broadcast updates.
async fn handle_socket<A: SessionService>(
    socket: WebSocket,
    rx: broadcast::Receiver<WsEvent>,
    auth_service: std::sync::Arc<A>,
    data_service: std::sync::Arc<data_service::PgDataService>,
) {
    let (mut sender, mut receiver) = socket.split();

    // Wait for auth message from client
    let auth_result = tokio::time::timeout(
        std::time::Duration::from_secs(AUTH_TIMEOUT_SECS),
        receiver.next(),
    )
    .await;

    let token = match auth_result {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Auth { token }) => token,
            Err(_) => {
                let _ = sender.close().await;
                return;
            }
        },
        _ => {
            let _ = sender.close().await;
            return;
        }
    };

    // Validate session token
    let session_id = match uuid::Uuid::parse_str(&token) {
        Ok(uuid) => auth::SessionId(uuid),
        Err(_) => {
            let _ = sender.close().await;
            return;
        }
    };

    let user_id = match auth_service.validate_session(session_id).await {
        Ok((user, _session)) => user.id,
        Err(_) => {
            let _ = sender.close().await;
            return;
        }
    };

    let gate: StoreGate = std::sync::Arc::new(move |store_id| {
        let data_service = data_service.clone();
        Box::pin(async move { may_see_store(&*data_service, user_id, store_id).await })
    });

    // Auth succeeded — hand off to the forwarding loop
    handle_socket_forwarding(sender, receiver, rx, gate).await;
}

/// Forward broadcast updates to an authenticated WebSocket client.
async fn handle_socket_forwarding(
    mut sender: futures::stream::SplitSink<WebSocket, Message>,
    receiver: futures::stream::SplitStream<WebSocket>,
    mut rx: broadcast::Receiver<WsEvent>,
    gate: StoreGate,
) {
    // Send connected acknowledgement. Serialising a unit-variant is infallible.
    #[allow(
        clippy::unwrap_used,
        reason = "serde_json of unit variant is infallible"
    )]
    let connected = serde_json::to_string(&StatusUpdate::Connected).unwrap();
    if sender.send(Message::Text(connected.into())).await.is_err() {
        return;
    }

    // Spawn a task to forward broadcast messages to the client
    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if !gate(event.store_id).await {
                continue;
            }
            let msg = match serde_json::to_string(&event.update) {
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
        let mut receiver = receiver;
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use axum::routing;
    use futures::StreamExt;

    fn store() -> StoreId {
        StoreId(uuid::Uuid::from_bytes([7; 16]))
    }

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
    fn test_client_message_auth_deserialize() {
        let json = r#"{"type":"auth","token":"abc-123"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::Auth { token } if token == "abc-123"));
    }

    #[test]
    fn test_client_message_missing_type_fails() {
        let json = r#"{"token":"abc-123"}"#;
        assert!(serde_json::from_str::<ClientMessage>(json).is_err());
    }

    #[test]
    fn test_ws_broadcast_send_no_receivers() {
        let broadcast = WsBroadcast::new(16);
        // Should not panic even with no receivers
        broadcast.send(store(), StatusUpdate::Ping);
    }

    #[tokio::test]
    async fn test_ws_broadcast_send_receive() {
        let broadcast = WsBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        broadcast.send(store(), StatusUpdate::Connected);
        broadcast.send(
            store(),
            StatusUpdate::InvoiceStatus {
                invoice_id: "inv_1".to_string(),
                status: "paid".to_string(),
            },
        );

        let msg1 = rx.recv().await.unwrap();
        assert!(matches!(msg1.update, StatusUpdate::Connected));

        let msg2 = rx.recv().await.unwrap();
        assert!(
            matches!(msg2.update, StatusUpdate::InvoiceStatus { invoice_id, .. } if invoice_id == "inv_1")
        );
    }

    #[tokio::test]
    async fn test_ws_broadcast_multiple_subscribers() {
        let broadcast = WsBroadcast::new(16);
        let mut rx1 = broadcast.subscribe();
        let mut rx2 = broadcast.subscribe();

        broadcast.send(store(), StatusUpdate::Ping);

        assert!(matches!(
            rx1.recv().await.unwrap().update,
            StatusUpdate::Ping
        ));
        assert!(matches!(
            rx2.recv().await.unwrap().update,
            StatusUpdate::Ping
        ));
    }

    #[tokio::test]
    async fn test_ws_broadcast_capacity_overflow_lags_receiver() {
        let broadcast = WsBroadcast::new(2);
        let mut rx = broadcast.subscribe();

        // Send more messages than the channel capacity
        broadcast.send(store(), StatusUpdate::Ping);
        broadcast.send(store(), StatusUpdate::Connected);
        broadcast.send(store(), StatusUpdate::Ping);

        // The receiver should report lagged (missed messages)
        let result = rx.recv().await;
        assert!(
            matches!(result, Err(broadcast::error::RecvError::Lagged(count)) if count > 0),
            "expected Lagged error, got {result:?}"
        );

        // After the lag error, the remaining buffered messages are still receivable
        let msg1 = rx.recv().await.unwrap();
        assert!(matches!(msg1.update, StatusUpdate::Connected));
        let msg2 = rx.recv().await.unwrap();
        assert!(matches!(msg2.update, StatusUpdate::Ping));
    }

    /// Helper handler for transport tests — upgrades to WebSocket and delegates
    /// to `handle_socket_forwarding` with a gate that permits every store.
    /// Which stores a socket may see is covered through the real `/ws` handler
    /// in `server/tests/ws_store_scope.rs`.
    async fn test_upgrade(
        ws: WebSocketUpgrade,
        axum::extract::State(bc): axum::extract::State<WsBroadcast>,
    ) -> impl IntoResponse {
        let rx = bc.subscribe();
        ws.on_upgrade(move |socket| {
            let (sender, receiver) = socket.split();
            let allow_all: StoreGate = std::sync::Arc::new(|_| Box::pin(async { true }));
            handle_socket_forwarding(sender, receiver, rx, allow_all)
        })
    }

    /// Spin up a one-shot axum server that exposes `handle_socket` at `/ws`.
    /// Returns the server address and the `WsBroadcast` sender.
    async fn spawn_test_ws_server() -> (std::net::SocketAddr, WsBroadcast) {
        let broadcast = WsBroadcast::new(16);
        let app = axum::Router::new()
            .route("/ws", routing::get(test_upgrade))
            .with_state(broadcast.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (addr, broadcast)
    }

    #[tokio::test]
    async fn test_handle_socket_sends_connected_on_open() {
        let (addr, _broadcast) = spawn_test_ws_server().await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("client connect failed");

        // First message must be the Connected acknowledgement
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for Connected")
            .unwrap()
            .unwrap();
        let update: StatusUpdate = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert!(matches!(update, StatusUpdate::Connected));
    }

    #[tokio::test]
    async fn test_handle_socket_forwards_broadcast() {
        let timeout = std::time::Duration::from_secs(5);
        let (addr, broadcast) = spawn_test_ws_server().await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("client connect failed");

        // Consume the Connected message
        tokio::time::timeout(timeout, ws.next())
            .await
            .expect("timed out waiting for Connected")
            .unwrap()
            .unwrap();

        // Broadcast a status update
        broadcast.send(
            store(),
            StatusUpdate::InvoiceStatus {
                invoice_id: "inv_42".to_string(),
                status: "paid".to_string(),
            },
        );

        let msg = tokio::time::timeout(timeout, ws.next())
            .await
            .expect("timed out waiting for broadcast")
            .unwrap()
            .unwrap();
        let update: StatusUpdate = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert!(
            matches!(update, StatusUpdate::InvoiceStatus { ref invoice_id, ref status }
                if invoice_id == "inv_42" && status == "paid")
        );
    }

    #[tokio::test]
    async fn test_handle_socket_client_close() {
        let (addr, broadcast) = spawn_test_ws_server().await;

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("client connect failed");

        // Consume Connected
        tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for Connected")
            .unwrap()
            .unwrap();

        // Client sends close frame
        ws.close(None).await.unwrap();

        // Server should handle the close gracefully — sending after close
        // should not panic (the broadcast just goes nowhere).
        broadcast.send(store(), StatusUpdate::Ping);
    }

    /// Verify the exact JSON contract that the client crate relies on.
    /// Both server and client define identical `StatusUpdate` enums. This test
    /// asserts the canonical JSON so any serde-attribute drift is caught.
    #[test]
    fn test_status_update_json_contract() {
        // Connected
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Connected).unwrap(),
            r#"{"type":"connected"}"#,
        );

        // Ping
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Ping).unwrap(),
            r#"{"type":"ping"}"#,
        );

        // InvoiceStatus
        let invoice = StatusUpdate::InvoiceStatus {
            invoice_id: "inv_1".to_string(),
            status: "paid".to_string(),
        };
        let json = serde_json::to_value(&invoice).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "invoice_status",
                "invoice_id": "inv_1",
                "status": "paid"
            })
        );

        // PaymentUpdate with amount
        let payment = StatusUpdate::PaymentUpdate {
            payment_id: "pay_1".to_string(),
            invoice_id: "inv_1".to_string(),
            status: "confirmed".to_string(),
            amount: Some("1.5".to_string()),
        };
        let json = serde_json::to_value(&payment).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "payment_update",
                "payment_id": "pay_1",
                "invoice_id": "inv_1",
                "status": "confirmed",
                "amount": "1.5"
            })
        );

        // PaymentUpdate without amount
        let payment_no_amt = StatusUpdate::PaymentUpdate {
            payment_id: "pay_2".to_string(),
            invoice_id: "inv_2".to_string(),
            status: "detecting".to_string(),
            amount: None,
        };
        let json = serde_json::to_value(&payment_no_amt).unwrap();
        // Use .get().unwrap().is_null() instead of json["amount"] == Null
        // so this assertion catches a future skip_serializing_if annotation
        // that would omit the key entirely.
        assert!(json.get("amount").unwrap().is_null());
    }
}
