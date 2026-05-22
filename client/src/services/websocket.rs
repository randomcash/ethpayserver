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

/// WebSocket connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected and not attempting to connect.
    Disconnected,
    /// Actively connected to the server.
    Connected,
    /// Connection lost, attempting to reconnect.
    Reconnecting,
}

/// Maximum reconnection delay in milliseconds.
const MAX_RECONNECT_DELAY_MS: u32 = 30_000;

/// Base reconnection delay in milliseconds.
const BASE_RECONNECT_DELAY_MS: u32 = 1_000;

/// WebSocket service providing reactive connection state and message handling.
///
/// Instantiate once in `ProtectedLayout` and provide via Leptos context.
/// Supports automatic exponential-backoff reconnection.
///
/// Usage:
/// ```rust,ignore
/// let ws = WebSocketService::new();
/// ws.connect("ws://localhost:5000/ws", Some("session_token"));
/// // Read ws.connection_state() signal, ws.last_update() signal
/// ```
pub struct WebSocketService {
    ws: Rc<RefCell<Option<WebSocket>>>,
    url: Rc<RefCell<Option<String>>>,
    token: Rc<RefCell<Option<String>>>,
    connection_state: ReadSignal<ConnectionState>,
    set_connection_state: WriteSignal<ConnectionState>,
    last_update: ReadSignal<Option<StatusUpdate>>,
    set_last_update: WriteSignal<Option<StatusUpdate>>,
    reconnect_attempts: Rc<RefCell<u32>>,
    reconnect_handle: Rc<RefCell<Option<i32>>>,
    intentional_disconnect: Rc<RefCell<bool>>,
}

impl Default for WebSocketService {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketService {
    /// Create a new disconnected WebSocket service.
    pub fn new() -> Self {
        let (connection_state, set_connection_state) = signal(ConnectionState::Disconnected);
        let (last_update, set_last_update) = signal(None::<StatusUpdate>);
        Self {
            ws: Rc::new(RefCell::new(None)),
            url: Rc::new(RefCell::new(None)),
            token: Rc::new(RefCell::new(None)),
            connection_state,
            set_connection_state,
            last_update,
            set_last_update,
            reconnect_attempts: Rc::new(RefCell::new(0)),
            reconnect_handle: Rc::new(RefCell::new(None)),
            intentional_disconnect: Rc::new(RefCell::new(false)),
        }
    }

    /// Current connection state.
    pub fn connection_state(&self) -> ReadSignal<ConnectionState> {
        self.connection_state
    }

    /// The last status update received.
    pub fn last_update(&self) -> ReadSignal<Option<StatusUpdate>> {
        self.last_update
    }

    /// Connect to the WebSocket endpoint with automatic reconnection.
    ///
    /// If a token is provided, it is sent as the first message after connection
    /// (not in the URL) to avoid leaking credentials in browser history, server
    /// logs, and proxy logs. Pass `None` for public endpoints (e.g. checkout).
    ///
    /// Closes any existing connection before opening the new one.
    pub fn connect(&self, url: &str, token: Option<&str>) -> Result<(), String> {
        // Close existing connection without triggering reconnect.
        if let Some(ws) = self.ws.borrow_mut().take() {
            *self.intentional_disconnect.borrow_mut() = true;
            let _ = ws.close();
        }
        if let Some(handle) = self.reconnect_handle.borrow_mut().take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(handle);
        }

        *self.intentional_disconnect.borrow_mut() = false;
        *self.url.borrow_mut() = Some(url.to_string());
        *self.token.borrow_mut() = token.map(|t| t.to_string());
        *self.reconnect_attempts.borrow_mut() = 0;
        self.connect_inner(url, token)
    }

    /// Internal connection logic shared by initial connect and reconnect.
    fn connect_inner(&self, url: &str, token: Option<&str>) -> Result<(), String> {
        use wasm_bindgen::closure::Closure;
        use web_sys::Event;

        let ws = WebSocket::new(url).map_err(|_| "Failed to create WebSocket".to_string())?;

        let ws_clone = ws.clone();

        // On open — reset reconnect counter, send auth message if token provided
        let set_state = self.set_connection_state;
        let reconnect_attempts = self.reconnect_attempts.clone();
        let ws_for_auth = ws.clone();
        let auth_token = token.map(|t| t.to_string());
        let onopen = Closure::once(move |_: Event| {
            *reconnect_attempts.borrow_mut() = 0;
            if let Some(ref t) = auth_token {
                let auth_msg = format!(r#"{{"type":"auth","token":"{}"}}"#, t);
                let _ = ws_for_auth.send_with_str(&auth_msg);
            }
            set_state.set(ConnectionState::Connected);
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        // On close — schedule reconnect unless intentionally disconnected
        let set_state = self.set_connection_state;
        let url_rc = self.url.clone();
        let token_rc = self.token.clone();
        let ws_rc = self.ws.clone();
        let reconnect_attempts = self.reconnect_attempts.clone();
        let reconnect_handle = self.reconnect_handle.clone();
        let intentional = self.intentional_disconnect.clone();
        let set_last_update = self.set_last_update;
        let onclose = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
            // Clear the stored socket
            ws_rc.borrow_mut().take();

            if *intentional.borrow() {
                set_state.set(ConnectionState::Disconnected);
                return;
            }

            set_state.set(ConnectionState::Reconnecting);

            // Exponential backoff: base * 2^attempts, capped at max
            let attempts = *reconnect_attempts.borrow();
            let delay = BASE_RECONNECT_DELAY_MS
                .saturating_mul(2u32.saturating_pow(attempts))
                .min(MAX_RECONNECT_DELAY_MS);
            *reconnect_attempts.borrow_mut() = attempts.saturating_add(1);

            let url_for_reconnect = url_rc.clone();
            let token_for_reconnect = token_rc.clone();
            let ws_storage = ws_rc.clone();
            let set_state_inner = set_state;
            let set_last_update_inner = set_last_update;
            let reconnect_attempts_inner = reconnect_attempts.clone();
            let reconnect_handle_inner = reconnect_handle.clone();
            let intentional_inner = intentional.clone();

            let closure = Closure::once(move || {
                // Clear the stored handle
                reconnect_handle_inner.borrow_mut().take();

                let url_opt = url_for_reconnect.borrow().clone();
                let token_opt = token_for_reconnect.borrow().clone();
                if let Some(ref url) = url_opt {
                    reconnect_one(
                        url,
                        token_opt.as_deref(),
                        set_state_inner,
                        set_last_update_inner,
                        url_for_reconnect.clone(),
                        token_for_reconnect.clone(),
                        ws_storage,
                        reconnect_attempts_inner,
                        intentional_inner,
                    );
                }
            });

            let window = web_sys::window().unwrap();
            let handle = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    delay as i32,
                )
                .unwrap_or(0);
            *reconnect_handle.borrow_mut() = Some(handle);
            closure.forget();
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();

        // On message — parse StatusUpdate, handle Ping/Connected silently
        let set_state = self.set_connection_state;
        let set_update = self.set_last_update;
        let onmessage = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            if let Some(text) = event.data().as_string()
                && let Ok(update) = serde_json::from_str::<StatusUpdate>(&text)
            {
                match &update {
                    StatusUpdate::Connected => {
                        set_state.set(ConnectionState::Connected);
                    }
                    StatusUpdate::Ping => {
                        // Keep-alive — no UI update needed
                    }
                    _ => {
                        set_update.set(Some(update));
                    }
                }
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        // On error
        let set_state = self.set_connection_state;
        let onerror = Closure::wrap(Box::new(move |_: Event| {
            set_state.set(ConnectionState::Reconnecting);
            leptos::logging::warn!("WebSocket error");
        }) as Box<dyn FnMut(Event)>);
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        *self.ws.borrow_mut() = Some(ws_clone);
        Ok(())
    }

    /// Disconnect from WebSocket. Suppresses automatic reconnection.
    pub fn disconnect(&self) {
        *self.intentional_disconnect.borrow_mut() = true;

        // Cancel any pending reconnect timer
        if let Some(handle) = self.reconnect_handle.borrow_mut().take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(handle);
        }

        if let Some(ws) = self.ws.borrow_mut().take() {
            let _ = ws.close();
        }
        self.set_connection_state.set(ConnectionState::Disconnected);
        *self.url.borrow_mut() = None;
        *self.token.borrow_mut() = None;
        *self.reconnect_attempts.borrow_mut() = 0;
    }
}

impl Drop for WebSocketService {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// Create a single reconnection WebSocket (used from the onclose callback).
///
/// Fixes RCS-131: the new WebSocket is now stored in `ws_storage` so it can
/// be closed by `disconnect()`. Previously the socket was created but never
/// stored, leaving orphaned connections after network blips.
#[allow(clippy::too_many_arguments)]
fn reconnect_one(
    url: &str,
    token: Option<&str>,
    set_state: WriteSignal<ConnectionState>,
    set_update: WriteSignal<Option<StatusUpdate>>,
    url_rc: Rc<RefCell<Option<String>>>,
    token_rc: Rc<RefCell<Option<String>>>,
    ws_storage: Rc<RefCell<Option<WebSocket>>>,
    reconnect_attempts: Rc<RefCell<u32>>,
    intentional: Rc<RefCell<bool>>,
) {
    use wasm_bindgen::closure::Closure;
    use web_sys::Event;

    let ws = match WebSocket::new(url) {
        Ok(ws) => ws,
        Err(_) => {
            // Schedule another reconnect
            set_state.set(ConnectionState::Reconnecting);
            let attempts = *reconnect_attempts.borrow();
            let delay = BASE_RECONNECT_DELAY_MS
                .saturating_mul(2u32.saturating_pow(attempts))
                .min(MAX_RECONNECT_DELAY_MS);
            *reconnect_attempts.borrow_mut() = attempts.saturating_add(1);

            let url_rc2 = url_rc.clone();
            let token_rc2 = token_rc.clone();
            let ws_storage2 = ws_storage.clone();
            let ra2 = reconnect_attempts.clone();
            let int2 = intentional.clone();
            let closure = Closure::once(move || {
                let url_opt = url_rc2.borrow().clone();
                let token_opt = token_rc2.borrow().clone();
                if let Some(ref u) = url_opt {
                    reconnect_one(
                        u,
                        token_opt.as_deref(),
                        set_state,
                        set_update,
                        url_rc2.clone(),
                        token_rc2.clone(),
                        ws_storage2,
                        ra2,
                        int2,
                    );
                }
            });
            let window = web_sys::window().unwrap();
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                delay as i32,
            );
            closure.forget();
            return;
        }
    };

    // Store the new WebSocket so disconnect() can close it (RCS-131 fix)
    *ws_storage.borrow_mut() = Some(ws.clone());

    // On open — send auth message if token provided, reset reconnect counter
    let ra_open = reconnect_attempts.clone();
    let ws_for_auth = ws.clone();
    let auth_token = token.map(|t| t.to_string());
    let onopen = Closure::once(move |_: Event| {
        *ra_open.borrow_mut() = 0;
        if let Some(ref t) = auth_token {
            let auth_msg = format!(r#"{{"type":"auth","token":"{}"}}"#, t);
            let _ = ws_for_auth.send_with_str(&auth_msg);
        }
        set_state.set(ConnectionState::Connected);
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    // On close — schedule reconnect
    let url_rc2 = url_rc.clone();
    let token_rc2 = token_rc.clone();
    let ws_storage2 = ws_storage.clone();
    let ra2 = reconnect_attempts.clone();
    let int2 = intentional.clone();
    let onclose = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
        ws_storage2.borrow_mut().take();

        if *int2.borrow() {
            set_state.set(ConnectionState::Disconnected);
            return;
        }
        set_state.set(ConnectionState::Reconnecting);

        let attempts = *ra2.borrow();
        let delay = BASE_RECONNECT_DELAY_MS
            .saturating_mul(2u32.saturating_pow(attempts))
            .min(MAX_RECONNECT_DELAY_MS);
        *ra2.borrow_mut() = attempts.saturating_add(1);

        let url_rc3 = url_rc2.clone();
        let token_rc3 = token_rc2.clone();
        let ws_storage3 = ws_storage2.clone();
        let ra3 = ra2.clone();
        let int3 = int2.clone();
        let closure = Closure::once(move || {
            let url_opt = url_rc3.borrow().clone();
            let token_opt = token_rc3.borrow().clone();
            if let Some(ref u) = url_opt {
                reconnect_one(
                    u,
                    token_opt.as_deref(),
                    set_state,
                    set_update,
                    url_rc3.clone(),
                    token_rc3.clone(),
                    ws_storage3,
                    ra3,
                    int3,
                );
            }
        });
        let window = web_sys::window().unwrap();
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            delay as i32,
        );
        closure.forget();
    }) as Box<dyn FnMut(web_sys::CloseEvent)>);
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    // On message
    let onmessage = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
        if let Some(text) = event.data().as_string()
            && let Ok(update) = serde_json::from_str::<StatusUpdate>(&text)
        {
            match &update {
                StatusUpdate::Connected => {
                    set_state.set(ConnectionState::Connected);
                }
                StatusUpdate::Ping => {}
                _ => {
                    set_update.set(Some(update));
                }
            }
        }
    }) as Box<dyn FnMut(web_sys::MessageEvent)>);
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // On error
    let onerror = Closure::wrap(Box::new(move |_: Event| {
        leptos::logging::warn!("WebSocket reconnection error");
    }) as Box<dyn FnMut(Event)>);
    ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();
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

    /// Mirror of the server-side `test_status_update_json_contract` test.
    /// Both tests assert the exact same JSON so any serde-attribute drift
    /// between client and server `StatusUpdate` definitions is caught.
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

    #[test]
    fn test_connection_state_default() {
        assert_eq!(ConnectionState::Disconnected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Connected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Reconnecting, ConnectionState::Connected);
    }

    #[test]
    fn test_reconnect_delay_calculation() {
        // Verify the exponential backoff formula
        let base = BASE_RECONNECT_DELAY_MS;
        let max = MAX_RECONNECT_DELAY_MS;

        assert_eq!(base.saturating_mul(2u32.saturating_pow(0)).min(max), 1_000);
        assert_eq!(base.saturating_mul(2u32.saturating_pow(1)).min(max), 2_000);
        assert_eq!(base.saturating_mul(2u32.saturating_pow(2)).min(max), 4_000);
        assert_eq!(base.saturating_mul(2u32.saturating_pow(3)).min(max), 8_000);
        assert_eq!(base.saturating_mul(2u32.saturating_pow(4)).min(max), 16_000);
        assert_eq!(base.saturating_mul(2u32.saturating_pow(5)).min(max), 30_000);
        assert_eq!(base.saturating_mul(2u32.saturating_pow(6)).min(max), 30_000);
    }
}
