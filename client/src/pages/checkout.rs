//! Public checkout page for customer payments.
//!
//! Displayed at `/checkout/{invoice_id}`. No authentication required.
//! Shows invoice amount, QR code, chain/asset selector, countdown timer,
//! and real-time payment status via WebSocket.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::api::{ApiError, CheckoutResponse, EvmApiClient, PaymentOption};
use crate::pages::invoices::chain_name;
use crate::services::websocket::{StatusUpdate, WebSocketService};

/// Format a human-readable amount from smallest units.
///
/// e.g., "1000000" with 6 decimals -> "1.000000"
fn format_crypto_amount(smallest_units: &str, decimals: u8) -> String {
    if decimals == 0 {
        return smallest_units.to_string();
    }
    let s = smallest_units.to_string();
    let len = s.len();
    let d = decimals as usize;
    if len <= d {
        let zeros = "0".repeat(d - len);
        format!("0.{}{}", zeros, s.trim_end_matches('0'))
    } else {
        let (int_part, frac_part) = s.split_at(len - d);
        let trimmed = frac_part.trim_end_matches('0');
        if trimmed.is_empty() {
            int_part.to_string()
        } else {
            format!("{}.{}", int_part, trimmed)
        }
    }
}

/// Public checkout page.
#[component]
pub fn CheckoutPage() -> impl IntoView {
    let params = use_params_map();
    let invoice_id = move || params.get().get("id").unwrap_or_default();

    // Create unauthenticated API client
    let api = EvmApiClient::new("");

    // Selected payment option index
    let (selected_idx, set_selected_idx) = signal(0usize);

    // Clipboard feedback
    let (copied, set_copied) = signal(false);

    // Refresh counter for manual retry / WS-triggered refresh
    let (refresh, set_refresh) = signal(0u32);

    let checkout_resource = LocalResource::new(move || {
        let api = api.clone();
        let id = invoice_id();
        let _ = refresh.get();
        async move {
            if id.is_empty() {
                return Err(ApiError::Network("No invoice ID".to_string()));
            }
            api.get_checkout(&id).await
        }
    });

    // WebSocket for real-time updates
    let ws = std::rc::Rc::new(WebSocketService::new());
    let ws_connected = ws.connected();
    let ws_update = ws.last_update();

    // Connect WebSocket on mount
    let id_for_ws = invoice_id();
    if !id_for_ws.is_empty() {
        let protocol = if web_sys::window()
            .and_then(|w| w.location().protocol().ok())
            .as_deref()
            == Some("https:")
        {
            "wss"
        } else {
            "ws"
        };
        let host = web_sys::window()
            .and_then(|w| w.location().host().ok())
            .unwrap_or_default();
        let ws_url = format!(
            "{}://{}/checkout/ws?invoice_id={}",
            protocol, host, id_for_ws
        );
        let _ = ws.connect(&ws_url);
    }

    // Clean up WebSocket on unmount
    let ws_cleanup = ws.clone();
    on_cleanup(move || {
        ws_cleanup.disconnect();
    });

    // Refresh data when a relevant WS update arrives
    Effect::new(move || {
        if let Some(update) = ws_update.get() {
            match update {
                StatusUpdate::InvoiceStatus { .. } | StatusUpdate::PaymentUpdate { .. } => {
                    set_refresh.update(|n| *n += 1);
                }
                _ => {}
            }
        }
    });

    view! {
        <div class="checkout-page">
            <div class="checkout-container">
                <div class="checkout-header">
                    <span class="checkout-logo">"E"</span>
                    <span class="checkout-brand">"ETHPayServer"</span>
                </div>

                <Suspense fallback=move || view! {
                    <div class="checkout-loading">
                        <div class="loading-spinner"></div>
                        <p>"Loading checkout..."</p>
                    </div>
                }>
                    {move || checkout_resource.get().map(|result| match &*result {
                        Err(e) => view! {
                            <div class="checkout-error">
                                <h2>"Invoice not found"</h2>
                                <p>{e.to_string()}</p>
                            </div>
                        }.into_any(),
                        Ok(data) => {
                            let data = data.clone();
                            render_checkout(data, selected_idx, set_selected_idx, copied, set_copied)
                        }
                    })}
                </Suspense>

                <div class="checkout-footer">
                    <span class="checkout-ws-status">
                        {move || if ws_connected.get() { "Live" } else { "" }}
                    </span>
                    <span class="checkout-powered">"Powered by ETHPayServer"</span>
                </div>
            </div>
        </div>
    }
}

/// Render the checkout content once data is loaded.
fn render_checkout(
    data: CheckoutResponse,
    selected_idx: ReadSignal<usize>,
    set_selected_idx: WriteSignal<usize>,
    copied: ReadSignal<bool>,
    set_copied: WriteSignal<bool>,
) -> AnyView {
    let status = data.status.clone();

    // Terminal states
    if data.is_paid {
        return view! {
            <div class="checkout-status checkout-paid">
                <div class="checkout-status-icon">"&#10003;"</div>
                <h2>"Payment Complete"</h2>
                <p class="checkout-amount">{data.amount.clone()}" "{data.currency.clone()}</p>
                <p class="checkout-status-detail">"Thank you for your payment."</p>
            </div>
        }
        .into_any();
    }

    if data.is_expired {
        return view! {
            <div class="checkout-status checkout-expired">
                <div class="checkout-status-icon">"&#10007;"</div>
                <h2>"Invoice Expired"</h2>
                <p class="checkout-amount">{data.amount.clone()}" "{data.currency.clone()}</p>
                <p class="checkout-status-detail">"This invoice is no longer accepting payments."</p>
            </div>
        }
        .into_any();
    }

    if status == "cancelled" {
        return view! {
            <div class="checkout-status checkout-expired">
                <div class="checkout-status-icon">"&#10007;"</div>
                <h2>"Invoice Cancelled"</h2>
                <p class="checkout-amount">{data.amount.clone()}" "{data.currency.clone()}</p>
            </div>
        }
        .into_any();
    }

    // Active payment states
    let active_options: Vec<PaymentOption> = data
        .payment_options
        .iter()
        .filter(|o| o.is_active)
        .cloned()
        .collect();

    if active_options.is_empty() {
        return view! {
            <div class="checkout-status checkout-expired">
                <h2>"No Payment Options"</h2>
                <p>"This invoice has no active payment methods configured."</p>
            </div>
        }
        .into_any();
    }

    let options = active_options.clone();
    let options_for_selector = active_options.clone();
    let amount = data.amount.clone();
    let currency = data.currency.clone();
    let expires_at = data.expires_at.clone();

    let status_label = match status.as_str() {
        "processing" => "Payment detected, awaiting confirmation...",
        "partially_paid" => "Partial payment received",
        _ => "Awaiting payment",
    };

    view! {
        <div class="checkout-body">
            // Amount and status
            <div class="checkout-amount-section">
                <p class="checkout-amount">{amount.clone()}" "{currency.clone()}</p>
                <p class="checkout-status-label">{status_label}</p>
                <CountdownTimer expires_at=expires_at />
            </div>

            // Chain/asset selector
            {if options_for_selector.len() > 1 {
                let opts = options_for_selector.clone();
                Some(view! {
                    <div class="checkout-chain-selector">
                        <label>"Pay with"</label>
                        <div class="checkout-chain-options">
                            {opts.into_iter().enumerate().map(|(i, opt)| {
                                let label = format!("{} ({})", opt.asset_symbol, chain_name(opt.chain_id));
                                view! {
                                    <button
                                        class=move || if selected_idx.get() == i { "chain-option active" } else { "chain-option" }
                                        on:click=move |_| set_selected_idx.set(i)
                                    >
                                        {label}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                })
            } else {
                None
            }}

            // QR code and payment details
            {move || {
                let idx = selected_idx.get();
                let opt = options.get(idx).or(options.first());
                opt.map(|option| {
                    let addr = option.payment_address.clone();
                    let display_amount = format_crypto_amount(&option.amount, option.decimals);
                    let asset = option.asset_symbol.clone();
                    let chain = chain_name(option.chain_id);
                    let addr_for_copy = addr.clone();

                    view! {
                        <div class="checkout-payment-details">
                            // QR code (address as text for now — ui-kit QrCodeCard needs WASM context)
                            <div class="checkout-qr">
                                <div class="checkout-qr-placeholder">
                                    <span class="checkout-qr-label">"Scan to pay"</span>
                                    <div class="checkout-qr-box">
                                        // QR code rendered via CSS background or JS in production.
                                        // For now, show the address prominently.
                                        <span class="checkout-qr-addr-short">{
                                            if addr.len() >= 10 {
                                                format!("{}...{}", &addr[..6], &addr[addr.len()-4..])
                                            } else {
                                                addr.clone()
                                            }
                                        }</span>
                                    </div>
                                </div>
                            </div>

                            // Amount in crypto
                            <div class="checkout-detail-row">
                                <span class="checkout-detail-label">"Amount"</span>
                                <span class="checkout-detail-value">{display_amount.clone()}" "{asset.clone()}</span>
                            </div>

                            // Network
                            <div class="checkout-detail-row">
                                <span class="checkout-detail-label">"Network"</span>
                                <span class="checkout-detail-value">{chain}</span>
                            </div>

                            // Address with copy
                            <div class="checkout-address-row">
                                <span class="checkout-detail-label">"Address"</span>
                                <div class="checkout-address-value">
                                    <code class="checkout-address">{addr.clone()}</code>
                                    <button
                                        class="checkout-copy-btn"
                                        on:click=move |_| {
                                            if let Some(window) = web_sys::window() {
                                                let clipboard = window.navigator().clipboard();
                                                let addr = addr_for_copy.clone();
                                                let _ = clipboard.write_text(&addr);
                                                set_copied.set(true);
                                                // Reset after 2 seconds
                                                leptos::task::spawn_local(async move {
                                                    gloo_timers::future::TimeoutFuture::new(2000).await;
                                                    set_copied.set(false);
                                                });
                                            }
                                        }
                                    >
                                        {move || if copied.get() { "Copied!" } else { "Copy" }}
                                    </button>
                                </div>
                            </div>
                        </div>
                    }
                })
            }}
        </div>
    }
    .into_any()
}

/// Countdown timer component.
#[component]
fn CountdownTimer(expires_at: String) -> impl IntoView {
    let (remaining, set_remaining) = signal(String::new());

    // Parse expiration and tick every second
    let expires = expires_at.clone();
    let handle = StoredValue::new(None::<gloo_timers::callback::Interval>);
    Effect::new(move |_| {
        let expires = expires.clone();
        let interval = gloo_timers::callback::Interval::new(1000, move || {
            let exp = js_sys::Date::parse(&expires);
            let now = js_sys::Date::now();
            let diff_ms = exp - now;
            if diff_ms <= 0.0 {
                set_remaining.set("Expired".to_string());
            } else {
                let secs = (diff_ms / 1000.0) as u64;
                let mins = secs / 60;
                let s = secs % 60;
                if mins > 60 {
                    let hrs = mins / 60;
                    let m = mins % 60;
                    set_remaining.set(format!("{}h {}m {}s", hrs, m, s));
                } else {
                    set_remaining.set(format!("{}m {}s", mins, s));
                }
            }
        });
        handle.set_value(Some(interval));
    });

    // Drop interval on unmount
    on_cleanup(move || {
        handle.set_value(None);
    });

    view! {
        <div class="checkout-countdown">
            <span class="checkout-countdown-label">"Expires in "</span>
            <span class="checkout-countdown-time">{remaining}</span>
        </div>
    }
}
