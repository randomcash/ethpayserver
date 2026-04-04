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
