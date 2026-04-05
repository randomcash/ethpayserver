//! WebSocket service for real-time invoice and payment status updates.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;

/// Real-time status update message from server.
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

/// WebSocket service providing reactive connection state and message handling.
///
/// Usage:
/// ```rust,ignore
/// let ws = WebSocketService::new();
/// ws.connect("ws://localhost:5000/ws?token=session_id");
/// // Read ws.connected() signal, ws.last_update() signal
/// ```
pub struct WebSocketService {
    ws: Rc<RefCell<Option<WebSocket>>>,
    connected: ReadSignal<bool>,
    set_connected: WriteSignal<bool>,
    last_update: ReadSignal<Option<StatusUpdate>>,
    set_last_update: WriteSignal<Option<StatusUpdate>>,
}

impl Default for WebSocketService {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketService {
    /// Create a new disconnected WebSocket service.
    pub fn new() -> Self {
        let (connected, set_connected) = signal(false);
        let (last_update, set_last_update) = signal(None::<StatusUpdate>);
        Self {
            ws: Rc::new(RefCell::new(None)),
            connected,
            set_connected,
            last_update,
            set_last_update,
        }
    }

    /// Whether the WebSocket is currently connected.
    pub fn connected(&self) -> ReadSignal<bool> {
        self.connected
    }

    /// The last status update received.
    pub fn last_update(&self) -> ReadSignal<Option<StatusUpdate>> {
        self.last_update
    }

    /// Connect to the WebSocket endpoint.
    pub fn connect(&self, url: &str) -> Result<(), String> {
        use wasm_bindgen::closure::Closure;
        use web_sys::Event;

        let ws = WebSocket::new(url).map_err(|_| "Failed to create WebSocket".to_string())?;

        let ws_clone = ws.clone();

        // On open
        let set_conn = self.set_connected;
        let onopen = Closure::once(move |_: Event| {
            set_conn.set(true);
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        // On close
        let set_conn = self.set_connected;
        let onclose = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
            set_conn.set(false);
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();

        // On message
        let set_update = self.set_last_update;
        let onmessage = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            if let Some(text) = event.data().as_string()
                && let Ok(update) = serde_json::from_str::<StatusUpdate>(&text)
            {
                set_update.set(Some(update));
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        // On error
        let set_conn = self.set_connected;
        let onerror = Closure::wrap(Box::new(move |_: Event| {
            set_conn.set(false);
            leptos::logging::warn!("WebSocket error");
        }) as Box<dyn FnMut(Event)>);
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        *self.ws.borrow_mut() = Some(ws_clone);
        Ok(())
    }

    /// Disconnect from WebSocket.
    pub fn disconnect(&self) {
        if let Some(ws) = self.ws.borrow_mut().take() {
            let _ = ws.close();
        }
        self.set_connected.set(false);
    }
}

impl Drop for WebSocketService {
    fn drop(&mut self) {
        self.disconnect();
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
        let json = serde_json::to_string(&update).unwrap();
        let parsed: StatusUpdate = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(parsed, StatusUpdate::InvoiceStatus { invoice_id, status } if invoice_id == "inv_1" && status == "paid")
        );
    }

    #[test]
    fn test_status_update_serde_payment_update_with_amount() {
        let update = StatusUpdate::PaymentUpdate {
            payment_id: "pay_1".to_string(),
            invoice_id: "inv_1".to_string(),
            status: "confirmed".to_string(),
            amount: Some("1.5".to_string()),
        };
        let json = serde_json::to_value(&update).unwrap();
        assert_eq!(json["type"], "payment_update");
        assert_eq!(json["amount"], "1.5");
    }

    #[test]
    fn test_status_update_serde_payment_update_without_amount() {
        let update = StatusUpdate::PaymentUpdate {
            payment_id: "pay_2".to_string(),
            invoice_id: "inv_2".to_string(),
            status: "detecting".to_string(),
            amount: None,
        };
        let json = serde_json::to_value(&update).unwrap();
        assert!(json.get("amount").unwrap().is_null());
    }

    #[test]
    fn test_status_update_connected_and_ping() {
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Connected).unwrap(),
            r#"{"type":"connected"}"#
        );
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Ping).unwrap(),
            r#"{"type":"ping"}"#
        );
    }

    #[test]
    fn test_status_update_roundtrip_all_variants() {
        let variants = vec![
            StatusUpdate::InvoiceStatus {
                invoice_id: "i".to_string(),
                status: "s".to_string(),
            },
            StatusUpdate::PaymentUpdate {
                payment_id: "p".to_string(),
                invoice_id: "i".to_string(),
                status: "s".to_string(),
                amount: None,
            },
            StatusUpdate::Connected,
            StatusUpdate::Ping,
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let _: StatusUpdate = serde_json::from_str(&json).unwrap();
        }
    }
}
