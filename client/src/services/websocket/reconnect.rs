//! The reconnect path: creates a replacement socket from inside an `onclose`
//! callback, where the owning [`super::service::WebSocketService`] is no
//! longer reachable.

use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;

use super::backoff::reconnect_delay_ms;
use super::types::{ConnectionState, StatusUpdate};

/// Create a single reconnection WebSocket (used from the onclose callback).
///
/// Fixes RCS-131: the new WebSocket is now stored in `ws_storage` so it can
/// be closed by `disconnect()`. Previously the socket was created but never
/// stored, leaving orphaned connections after network blips.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconnect_one(
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
            let delay = reconnect_delay_ms(attempts);
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
        let delay = reconnect_delay_ms(attempts);
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
