//! The [`WebSocketService`] handle: reactive connection state plus the
//! connect/disconnect lifecycle. Held once in `ProtectedLayout` and shared
//! through Leptos context.

use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;

use super::backoff::reconnect_delay_ms;
use super::reconnect::reconnect_one;
use super::types::{ConnectionState, StatusUpdate};

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
            let delay = reconnect_delay_ms(attempts);
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
