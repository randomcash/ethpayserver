//! Countdown timer for the checkout page.

use leptos::prelude::*;
use send_wrapper::SendWrapper;

/// Countdown timer component.
#[component]
pub fn CountdownTimer(expires_at: String) -> impl IntoView {
    let (remaining, set_remaining) = signal(String::new());

    // Parse expiration and tick every second
    let expires = expires_at.clone();
    let interval_handle: std::rc::Rc<std::cell::RefCell<Option<gloo_timers::callback::Interval>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let handle_for_effect = interval_handle.clone();
    Effect::new(move |_| {
        let expires = expires.clone();
        let interval = gloo_timers::callback::Interval::new(1000, move || {
            let exp = js_sys::Date::parse(&expires);
            // Date::parse returns NaN for malformed input. Without this guard
            // `NaN - now = NaN`, `NaN <= 0.0` is false, and `NaN as u64 = 0`,
            // so the timer would silently show "0m 0s" instead of surfacing
            // the error. Surface it explicitly so we notice if the server
            // ever serialises expires_at in an unexpected shape.
            if exp.is_nan() {
                set_remaining.set("—".to_string());
                return;
            }
            let now = js_sys::Date::now();
            let diff_ms = exp - now;
            if diff_ms <= 0.0 {
                set_remaining.set("Expired".to_string());
                return;
            }
            let secs = (diff_ms / 1000.0) as u64;
            let days = secs / 86_400;
            let hrs = (secs % 86_400) / 3600;
            let mins = (secs % 3600) / 60;
            let s = secs % 60;
            let label = if days > 0 {
                format!("{days}d {hrs}h {mins}m")
            } else if hrs > 0 {
                format!("{hrs}h {mins}m {s}s")
            } else {
                format!("{mins}m {s}s")
            };
            set_remaining.set(label);
        });
        *handle_for_effect.borrow_mut() = Some(interval);
    });

    // Drop interval on unmount
    let handle_for_cleanup = SendWrapper::new(interval_handle);
    on_cleanup(move || {
        handle_for_cleanup.borrow_mut().take();
    });

    view! {
        <div class="checkout-countdown">
            <span class="checkout-countdown-label">"Expires in "</span>
            <span class="checkout-countdown-time">{remaining}</span>
        </div>
    }
}
