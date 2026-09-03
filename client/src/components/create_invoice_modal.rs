//! Create invoice modal form component.
//!
//! Shared modal that can be triggered from the header button or
//! the invoices page. Posts to `POST /invoices` and navigates
//! to the created invoice on success.

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use types::currency::{EXPIRATION_PRESETS, INVOICE_CURRENCY_OPTIONS};

use crate::api::{CreateInvoiceRequest, EvmApiClient};
use crate::app::{StoreContext, StoresStatus};

/// Shared signal that any component can use to open the create-invoice modal.
#[derive(Clone, Copy)]
pub struct CreateInvoiceSignal {
    pub show: ReadSignal<bool>,
    pub set_show: WriteSignal<bool>,
}

impl CreateInvoiceSignal {
    pub fn open(&self) {
        self.set_show.set(true);
    }
}

/// Create invoice modal form.
///
/// Reads `CreateInvoiceSignal` from context to determine visibility.
/// Reads `EvmApiClient` and `StoreContext` from context.
#[component]
pub fn CreateInvoiceModal() -> impl IntoView {
    let modal_signal =
        use_context::<CreateInvoiceSignal>().expect("CreateInvoiceSignal must be provided");
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");
    let navigate = use_navigate();

    // Signals off StoreContext are Copy, so grab them once instead of
    // cloning the whole context into every closure below.
    let stores = store_ctx.stores;
    let stores_status = store_ctx.stores_status;
    let selected_store_id = store_ctx.selected_store_id;

    // Form fields
    let default_currency = INVOICE_CURRENCY_OPTIONS[0].0.to_string();
    let default_expiration = EXPIRATION_PRESETS[0].0.to_string();
    // Store to invoice against. Empty = nothing picked yet; the modal owns this
    // rather than reading the global selection at submit time, so an invoice can
    // be created while the header sits on "All Stores" (RCS-172).
    let (store_id, set_store_id) = signal(String::new());
    let (amount, set_amount) = signal(String::new());
    let (currency, set_currency) = signal(default_currency.clone());
    let (expiration_minutes, set_expiration_minutes) = signal(default_expiration.clone());
    let (order_id, set_order_id) = signal(String::new());
    let (buyer_email, set_buyer_email) = signal(String::new());
    let (notification_url, set_notification_url) = signal(String::new());
    let (redirect_url, set_redirect_url) = signal(String::new());

    // UI state
    let (error, set_error) = signal(Option::<String>::None);
    let (submitting, set_submitting) = signal(false);
    let (show_advanced, set_show_advanced) = signal(false);

    // Seed the store picker on open, reset the form on close.
    Effect::new(move || {
        if modal_signal.show.get() {
            // Pre-fill with the globally selected store; on "All Stores" this
            // stays empty and the picker shows its placeholder. Read untracked
            // so switching the header store mid-edit doesn't clobber a choice
            // the user already made in the modal.
            set_store_id.set(selected_store_id.get_untracked().unwrap_or_default());
        } else {
            set_store_id.set(String::new());
            set_amount.set(String::new());
            set_currency.set(default_currency.clone());
            set_expiration_minutes.set(default_expiration.clone());
            set_order_id.set(String::new());
            set_buyer_email.set(String::new());
            set_notification_url.set(String::new());
            set_redirect_url.set(String::new());
            set_error.set(None);
            set_show_advanced.set(false);
        }
    });

    // A selection is valid only if it names a store actually in the fetched
    // list - non-emptiness is not enough. The seed above comes from persisted
    // state and is applied before the fetch resolves, so the signal can hold an
    // id the picker has no <option> for: the select renders blank while the
    // button stays enabled. That window is not just transient - on
    // StoresStatus::Failed the stale selection is never cleared (app/layout.rs
    // clears it only on the success path), so a failed store fetch leaves the
    // form submittable against a store the user cannot see and did not choose
    // this session. The server rejects it (crud.rs checks store permission
    // first), but the user gets an opaque 403 instead of a disabled button.
    let store_selection_valid = move || {
        let id = store_id.get();
        !id.is_empty() && stores.get().iter().any(|store| store.id == id)
    };

    let close = move || modal_signal.set_show.set(false);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let api = api.get();
        let store_id_val = store_id.get();
        let amount_val = amount.get().trim().to_string();
        let currency_val = currency.get();
        let exp_min = expiration_minutes.get();
        let order_id_val = order_id.get().trim().to_string();
        let buyer_email_val = buyer_email.get().trim().to_string();
        let notif_url = notification_url.get().trim().to_string();
        let redir_url = redirect_url.get().trim().to_string();
        let navigate = navigate.clone();

        set_error.set(None);

        // --- Client-side validation ---
        if !store_selection_valid() {
            set_error.set(Some("Please select a store".to_string()));
            return;
        }

        if amount_val.is_empty() {
            set_error.set(Some("Amount is required".to_string()));
            return;
        }

        // Validate amount is a positive finite number
        match amount_val.parse::<f64>() {
            Ok(n) if !n.is_finite() || n <= 0.0 => {
                set_error.set(Some("Amount must be greater than zero".to_string()));
                return;
            }
            Err(_) => {
                set_error.set(Some("Amount must be a valid number".to_string()));
                return;
            }
            _ => {}
        }

        // Validate URLs if provided
        if !notif_url.is_empty() && !is_valid_url(&notif_url) {
            set_error.set(Some(
                "Notification URL must be a valid HTTP(S) URL".to_string(),
            ));
            return;
        }
        if !redir_url.is_empty() && !is_valid_url(&redir_url) {
            set_error.set(Some("Redirect URL must be a valid HTTP(S) URL".to_string()));
            return;
        }

        // Build metadata from order_id and buyer_email
        let metadata = build_metadata(&order_id_val, &buyer_email_val);

        let exp_seconds = exp_min.parse::<u64>().ok().map(|m| m * 60);

        let request = CreateInvoiceRequest {
            store_id: store_id_val,
            currency: currency_val,
            amount: amount_val,
            expiration_seconds: exp_seconds,
            metadata,
            customer_email: if buyer_email_val.is_empty() {
                None
            } else {
                Some(buyer_email_val)
            },
            webhook_url: if notif_url.is_empty() {
                None
            } else {
                Some(notif_url)
            },
            redirect_url: if redir_url.is_empty() {
                None
            } else {
                Some(redir_url)
            },
        };

        set_submitting.set(true);

        leptos::task::spawn_local(async move {
            match api.create_invoice(&request).await {
                Ok(invoice) => {
                    modal_signal.set_show.set(false);
                    navigate(&format!("/evm/invoices/{}", invoice.id), Default::default());
                }
                Err(e) => {
                    set_error.set(Some(e.to_string()));
                }
            }
            set_submitting.set(false);
        });
    };

    // Placeholder doubles as the stores-fetch status line, so a slow or
    // failed fetch doesn't read as "this account has no stores" (RCS-195).
    let store_placeholder = move || match stores_status.get() {
        StoresStatus::Loading => "Loading stores...",
        StoresStatus::Failed(_) => "Could not load stores",
        StoresStatus::Loaded => "Select a store",
    };

    view! {
        <div
            class="modal-overlay"
            style:display=move || if modal_signal.show.get() { "flex" } else { "none" }
            on:click=move |_| close()
        >
            <div class="modal modal-md" on:click=|ev| ev.stop_propagation()>
                <div class="modal-header">
                    <h2>"Create Invoice"</h2>
                    <button class="btn btn-ghost btn-sm btn-icon" on:click=move |_| close()>
                        <IconClose />
                    </button>
                </div>
                <form on:submit=on_submit>
                    <div class="modal-body">
                        // Error banner
                        {move || error.get().map(|e| view! {
                            <div class="form-alert form-alert-error">{e}</div>
                        })}

                        // Store picker
                        <div class="form-group">
                            <label class="form-label" for="ci-store">"Store"</label>
                            <select
                                id="ci-store"
                                class="form-input"
                                prop:value=move || store_id.get()
                                on:change=move |ev| set_store_id.set(event_target_value(&ev))
                                required
                            >
                                <option value="" disabled selected=move || store_id.get().is_empty()>
                                    {store_placeholder}
                                </option>
                                // Re-rendered whenever the store list changes, so a
                                // store seeded before the fetch landed still ends up
                                // selected once its option exists.
                                {move || stores.get().into_iter().map(|store| {
                                    let selected = store_id.get_untracked() == store.id;
                                    view! { <option value=store.id selected=selected>{store.name}</option> }
                                }).collect_view()}
                            </select>
                        </div>

                        // Amount + Currency (side by side)
                        <div class="form-row">
                            <div class="form-group form-group-grow">
                                <label class="form-label" for="ci-amount">"Amount"</label>
                                <input
                                    type="text"
                                    inputmode="decimal"
                                    id="ci-amount"
                                    class="form-input"
                                    placeholder="0.00"
                                    prop:value=move || amount.get()
                                    on:input=move |ev| set_amount.set(event_target_value(&ev))
                                    required
                                />
                            </div>
                            <div class="form-group">
                                <label class="form-label" for="ci-currency">"Currency"</label>
                                <select
                                    id="ci-currency"
                                    class="form-input"
                                    prop:value=move || currency.get()
                                    on:change=move |ev| set_currency.set(event_target_value(&ev))
                                >
                                    {INVOICE_CURRENCY_OPTIONS.iter().map(|(value, label)| {
                                        view! { <option value=*value>{*label}</option> }
                                    }).collect_view()}
                                </select>
                            </div>
                        </div>

                        // Expiration
                        <div class="form-group">
                            <label class="form-label" for="ci-expiration">"Expiration"</label>
                            <select
                                id="ci-expiration"
                                class="form-input"
                                prop:value=move || expiration_minutes.get()
                                on:change=move |ev| set_expiration_minutes.set(event_target_value(&ev))
                            >
                                {EXPIRATION_PRESETS.iter().map(|(value, label)| {
                                    view! { <option value=*value>{*label}</option> }
                                }).collect_view()}
                            </select>
                            <p class="form-help">"Time until the invoice expires if unpaid."</p>
                        </div>

                        // Order ID
                        <div class="form-group">
                            <label class="form-label" for="ci-order-id">"Order ID "<span class="form-optional">"(optional)"</span></label>
                            <input
                                type="text"
                                id="ci-order-id"
                                class="form-input"
                                placeholder="e.g. ORD-12345"
                                prop:value=move || order_id.get()
                                on:input=move |ev| set_order_id.set(event_target_value(&ev))
                            />
                            <p class="form-help">"Your internal order reference. Stored in invoice metadata."</p>
                        </div>

                        // Buyer Email
                        <div class="form-group">
                            <label class="form-label" for="ci-buyer-email">"Buyer Email "<span class="form-optional">"(optional)"</span></label>
                            <input
                                type="email"
                                id="ci-buyer-email"
                                class="form-input"
                                placeholder="buyer@example.com"
                                prop:value=move || buyer_email.get()
                                on:input=move |ev| set_buyer_email.set(event_target_value(&ev))
                            />
                        </div>

                        // Advanced section (collapsed by default)
                        <div class="form-advanced-toggle">
                            <button
                                type="button"
                                class="btn btn-ghost btn-sm"
                                on:click=move |_| set_show_advanced.update(|v| *v = !*v)
                            >
                                <IconChevron expanded=show_advanced />
                                {move || if show_advanced.get() { "Hide advanced options" } else { "Show advanced options" }}
                            </button>
                        </div>

                        <Show when=move || show_advanced.get()>
                            <div class="form-advanced-section">
                                // Notification URL (Webhook)
                                <div class="form-group">
                                    <label class="form-label" for="ci-notification-url">"Notification URL"</label>
                                    <input
                                        type="url"
                                        id="ci-notification-url"
                                        class="form-input"
                                        placeholder="https://example.com/webhook"
                                        prop:value=move || notification_url.get()
                                        on:input=move |ev| set_notification_url.set(event_target_value(&ev))
                                    />
                                    <p class="form-help">"Receives a POST when the invoice status changes."</p>
                                </div>

                                // Redirect URL
                                <div class="form-group">
                                    <label class="form-label" for="ci-redirect-url">"Redirect URL"</label>
                                    <input
                                        type="url"
                                        id="ci-redirect-url"
                                        class="form-input"
                                        placeholder="https://example.com/thank-you"
                                        prop:value=move || redirect_url.get()
                                        on:input=move |ev| set_redirect_url.set(event_target_value(&ev))
                                    />
                                    <p class="form-help">"Customer is sent here after payment."</p>
                                </div>
                            </div>
                        </Show>
                    </div>
                    <div class="modal-footer">
                        <button type="button" class="btn btn-secondary btn-sm" on:click=move |_| close()>
                            "Cancel"
                        </button>
                        <button
                            type="submit"
                            class="btn btn-primary btn-sm"
                            disabled=move || submitting.get() || !store_selection_valid()
                        >
                            {move || if submitting.get() { "Creating..." } else { "Create Invoice" }}
                        </button>
                    </div>
                </form>
            </div>
        </div>
    }
}

/// Build metadata JSON from optional order_id and buyer_email fields.
fn build_metadata(order_id: &str, buyer_email: &str) -> Option<serde_json::Value> {
    if order_id.is_empty() && buyer_email.is_empty() {
        return None;
    }
    let mut map = serde_json::Map::new();
    if !order_id.is_empty() {
        map.insert(
            "order_id".to_string(),
            serde_json::Value::String(order_id.to_string()),
        );
    }
    if !buyer_email.is_empty() {
        map.insert(
            "buyer_email".to_string(),
            serde_json::Value::String(buyer_email.to_string()),
        );
    }
    Some(serde_json::Value::Object(map))
}

/// Basic URL validation: must start with http:// or https:// followed by content.
fn is_valid_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") && url.len() > "http://".len())
        || (lower.starts_with("https://") && url.len() > "https://".len())
}

// ===== Icons (local to this component) =====

#[component]
fn IconClose() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
    }
}

#[component]
fn IconChevron(expanded: ReadSignal<bool>) -> impl IntoView {
    view! {
        <svg
            xmlns="http://www.w3.org/2000/svg"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            style=move || if expanded.get() { "transform: rotate(180deg); transition: transform 0.15s" } else { "transition: transform 0.15s" }
        >
            <polyline points="6 9 12 15 18 9"></polyline>
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_metadata_both_fields() {
        let meta = build_metadata("ORD-123", "buyer@example.com");
        let meta = meta.expect("should be Some");
        assert_eq!(meta["order_id"], "ORD-123");
        assert_eq!(meta["buyer_email"], "buyer@example.com");
    }

    #[test]
    fn test_build_metadata_order_id_only() {
        let meta = build_metadata("ORD-456", "");
        let meta = meta.expect("should be Some");
        assert_eq!(meta["order_id"], "ORD-456");
        assert!(meta.get("buyer_email").is_none());
    }

    #[test]
    fn test_build_metadata_email_only() {
        let meta = build_metadata("", "test@example.com");
        let meta = meta.expect("should be Some");
        assert!(meta.get("order_id").is_none());
        assert_eq!(meta["buyer_email"], "test@example.com");
    }

    #[test]
    fn test_build_metadata_empty() {
        assert!(build_metadata("", "").is_none());
    }

    #[test]
    fn test_is_valid_url_accepts_http_and_https() {
        assert!(is_valid_url("https://example.com/webhook"));
        assert!(is_valid_url("http://localhost:3000/callback"));
        assert!(is_valid_url("https://api.example.com/v1/notify?token=abc"));
        assert!(is_valid_url("http://192.168.1.1:8080/hook"));
        assert!(is_valid_url(
            "https://example.com/path/to/resource#fragment"
        ));
    }

    #[test]
    fn test_is_valid_url_rejects_non_http_schemes() {
        assert!(!is_valid_url("ftp://files.example.com"));
        assert!(!is_valid_url("ws://example.com/ws"));
        assert!(!is_valid_url("wss://example.com/ws"));
        assert!(!is_valid_url("javascript:alert(1)"));
        assert!(!is_valid_url("data:text/html,<h1>hi</h1>"));
        assert!(!is_valid_url("file:///etc/passwd"));
    }

    #[test]
    fn test_is_valid_url_rejects_invalid_input() {
        assert!(!is_valid_url(""));
        assert!(!is_valid_url("not-a-url"));
        assert!(!is_valid_url("example.com"));
        assert!(!is_valid_url(" https://example.com"));
        assert!(!is_valid_url("http://"));
        assert!(!is_valid_url("https://"));
    }

    #[test]
    fn test_is_valid_url_accepts_case_insensitive_scheme() {
        assert!(is_valid_url("HTTP://EXAMPLE.COM"));
        assert!(is_valid_url("Https://Example.Com/path"));
    }
}
