//! Client-side error capture for the self-hosted errex (Sentry-protocol)
//! backend — the browser half of the telemetry the Rust binaries already send.
//!
//! # Reachability (read before enabling)
//!
//! errex is tailnet-only (its host is supplied at build time): an
//! end-user browser cannot reach it, and there is deliberately no public
//! ingress in front of errex ingest from this repo's side. Client capture is
//! therefore **off unless explicitly configured**, and is meant to be turned on
//! for internal/dev traffic (developer machines, the testnet VPS) where the
//! tailnet is reachable. When errex grows a public ingest hostname, pointing
//! the meta tag at it is the only change needed here.
//!
//! # Configuration
//!
//! Runtime, not compile-time — so a deployment can flip it without rebuilding
//! the WASM bundle. `client/index.html` carries:
//!
//! ```html
//! <meta name="errex-dsn" content="">
//! <meta name="errex-environment" content="">
//! ```
//!
//! An empty or malformed `errex-dsn` disables reporting entirely. `release` is
//! taken from `CI_COMMIT_SHORT_SHA` at build time, matching the server.
//!
//! # What is sent
//!
//! Panics (via the panic hook), uncaught JS errors and unhandled promise
//! rejections: a message, a raw stack trace (errex has no source maps yet) and
//! the route path. Never cookies, request bodies, user identity or query
//! strings — see the `event` module for the full payload and `scrub` for the
//! redaction shared with `evm::telemetry`.

mod dsn;
mod event;
mod scrub;

use std::cell::{Cell, RefCell};

use gloo_net::http::Request;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;

use dsn::Dsn;
use event::{Meta, Report};

/// Upper bound on events reported per page load.
///
/// A render loop that panics every frame would otherwise hammer errex ingest
/// (and the user's connection) until the tab is closed.
const MAX_EVENTS_PER_PAGE: u32 = 20;

thread_local! {
    /// `None` until [`init`] finds a usable DSN — which is also the permanent
    /// state whenever the meta tag is empty, i.e. the default.
    static CONFIG: RefCell<Option<Config>> = const { RefCell::new(None) };
    /// Events reported so far this page load.
    static SENT: Cell<u32> = const { Cell::new(0) };
}

/// Resolved reporting configuration.
struct Config {
    ingest_url: String,
    environment: Option<String>,
    release: Option<String>,
}

impl Config {
    /// Read the configuration off the document's meta tags.
    ///
    /// Returns `None` — reporting disabled — when the DSN is absent, empty or
    /// malformed.
    fn from_document() -> Option<Self> {
        let document = web_sys::window()?.document()?;
        Some(Self {
            ingest_url: Dsn::parse(&meta(&document, "errex-dsn")?)?.ingest_url,
            environment: meta(&document, "errex-environment"),
            release: option_env!("CI_COMMIT_SHORT_SHA").map(str::to_string),
        })
    }
}

/// Install the panic hook and the global error listeners.
///
/// A no-op when no DSN is configured, which is the default everywhere.
pub fn init() {
    let Some(config) = Config::from_document() else {
        return;
    };
    CONFIG.set(Some(config));
    install_panic_hook();
    install_global_listeners();
}

/// Report a Rust panic, keeping the console output the app had before.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // This replaces the hook `console_error_panic_hook::set_once`
        // installed, so call it explicitly rather than losing the console
        // message the developer expects to see.
        console_error_panic_hook::hook(info);
        let stack = stack_of(&js_sys::Error::new("").into());
        capture("panic", &info.to_string(), stack);
    }));
}

/// Report uncaught JS errors and unhandled promise rejections.
///
/// Both listeners take a plain [`web_sys::Event`] and downcast: the `error`
/// event also fires for failed resource loads, where the event is *not* an
/// `ErrorEvent` and reading `message` off it would panic inside the error
/// handler.
fn install_global_listeners() {
    let Some(window) = web_sys::window() else {
        return;
    };

    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(event) = event.dyn_ref::<web_sys::ErrorEvent>() else {
            return;
        };
        let (_, stack) = describe(&event.error());
        capture("error", &event.message(), stack);
    });
    let on_rejection = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(event) = event.dyn_ref::<web_sys::PromiseRejectionEvent>() else {
            return;
        };
        let (message, stack) = describe(&event.reason());
        capture("unhandledrejection", &message, stack);
    });

    let listeners = [
        ("error", on_error.as_ref()),
        ("unhandledrejection", on_rejection.as_ref()),
    ];
    for (name, callback) in listeners {
        let _ = window.add_event_listener_with_callback(name, callback.unchecked_ref());
    }
    // The listeners live for the life of the page.
    on_error.forget();
    on_rejection.forget();
}

/// Build and send one event, if reporting is on and the budget allows.
fn capture(kind: &str, message: &str, stack: Option<String>) {
    CONFIG.with_borrow(|config| {
        let Some(config) = config else {
            return;
        };
        if !take_budget() {
            return;
        }
        let body = event::envelope(
            &Report {
                kind,
                message,
                stack,
                route: route(),
            },
            &Meta {
                event_id: uuid::Uuid::new_v4().simple().to_string(),
                timestamp: js_sys::Date::now() / 1000.0,
                release: config.release.clone(),
                environment: config.environment.clone(),
            },
        );
        send(config.ingest_url.clone(), body);
    });
}

/// POST the envelope, fire and forget.
fn send(url: String, body: String) {
    spawn_local(async move {
        // Failures are swallowed on purpose: with errex on the tailnet an
        // unreachable ingest is the expected case in a public browser, and a
        // telemetry error must never become a user-visible one.
        let request = Request::post(&url)
            .header("Content-Type", "application/x-sentry-envelope")
            .body(body);
        if let Ok(request) = request {
            let _ = request.send().await;
        }
    });
}

/// Claim one slot from the per-page event budget.
fn take_budget() -> bool {
    SENT.with(|sent| {
        let count = sent.get();
        if count >= MAX_EVENTS_PER_PAGE {
            return false;
        }
        sent.set(count + 1);
        true
    })
}

/// Current route path. Query string and fragment are dropped — they carry
/// invoice/session identifiers and are never worth shipping.
fn route() -> Option<String> {
    web_sys::window()?.location().pathname().ok()
}

/// Content of `<meta name="{name}" content="…">`, if non-empty.
fn meta(document: &web_sys::Document, name: &str) -> Option<String> {
    let element = document
        .query_selector(&format!("meta[name=\"{name}\"]"))
        .ok()??;
    let content = element.get_attribute("content")?;
    let content = content.trim();
    (!content.is_empty()).then(|| content.to_string())
}

/// Read `Error.stack`. It is non-standard — hence no js-sys getter — but every
/// engine this app runs on implements it, and a raw trace is exactly what
/// errex wants until it grows source-map support.
fn stack_of(value: &JsValue) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str("stack"))
        .ok()
        .and_then(|stack| stack.as_string())
}

/// Pull a message and (where there is one) a stack out of a thrown value,
/// which JS allows to be literally anything.
fn describe(value: &JsValue) -> (String, Option<String>) {
    if let Some(error) = value.dyn_ref::<js_sys::Error>() {
        return (String::from(error.message()), stack_of(value));
    }
    if let Some(text) = value.as_string() {
        return (text, None);
    }
    let rendered = js_sys::JSON::stringify(value)
        .map(String::from)
        .unwrap_or_else(|_| "unserializable value".to_string());
    (rendered, None)
}
