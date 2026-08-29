//! Create invoice modal form component.
//!
//! Shared modal that can be triggered from the header button or
//! the invoices page. Posts to `POST /invoices` and navigates
//! to the created invoice on success.
//!
//! The form-field helpers live in [`helpers`] and the inline SVGs in
//! [`icons`]; this module holds the modal component itself.

mod helpers;
mod icons;

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use types::currency::{EXPIRATION_PRESETS, INVOICE_CURRENCY_OPTIONS};

use crate::api::{CreateInvoiceRequest, EvmApiClient};
use crate::app::StoreContext;

use helpers::{build_metadata, is_valid_url};
use icons::{IconChevron, IconClose, IconStore};

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

    // Form fields
    let default_currency = INVOICE_CURRENCY_OPTIONS[0].0.to_string();
    let default_expiration = EXPIRATION_PRESETS[0].0.to_string();
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

    // Reset form when modal closes
    Effect::new(move || {
        if !modal_signal.show.get() {
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

    let close = move || modal_signal.set_show.set(false);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let api = api.get();
        let store_id = store_ctx.selected_store_id.get();
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
        let Some(store_id) = store_id else {
            set_error.set(Some("Please select a store first".to_string()));
            return;
        };

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
            store_id,
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

    // Selected store name for display
    let store_name = {
        let store_ctx = store_ctx.clone();
        move || {
            store_ctx
                .selected_store()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "No store selected".to_string())
        }
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

                        // Store info
                        <div class="form-store-info">
                            <IconStore />
                            <span>{store_name}</span>
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
                            disabled=move || submitting.get()
                        >
                            {move || if submitting.get() { "Creating..." } else { "Create Invoice" }}
                        </button>
                    </div>
                </form>
            </div>
        </div>
    }
}
