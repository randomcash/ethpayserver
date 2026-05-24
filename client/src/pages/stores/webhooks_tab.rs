//! Webhooks tab component.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::{EvmApiClient, StoreWebhook, UpdateWebhookRequest};

use super::{IconWebhook, format_date};

/// Webhooks tab.
#[component]
pub fn WebhooksTab(store_id: String) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let (refresh_counter, set_refresh_counter) = signal(0u32);
    let store_id_fetch = store_id.clone();
    let webhook_resource = LocalResource::new(move || {
        let api = api.get();
        let sid = store_id_fetch.clone();
        refresh_counter.get();
        async move {
            match api.get_store_webhook(&sid).await {
                Ok(wh) => Ok(Some(wh)),
                Err(crate::api::ApiError::Http { status: 404, .. }) => Ok(None),
                Err(e) => Err(e),
            }
        }
    });

    // Configure form state
    let (show_form, set_show_form) = signal(false);
    let (form_url, set_form_url) = signal(String::new());
    let (form_enabled, set_form_enabled) = signal(true);
    let (saving, set_saving) = signal(false);
    let (save_error, set_save_error) = signal(None::<String>);
    // Store the secret after configure (only shown once)
    let (revealed_secret, set_revealed_secret) = signal(None::<String>);

    let store_id_save = store_id.clone();
    let on_save = move |_| {
        let url = form_url.get_untracked();
        if url.trim().is_empty() {
            set_save_error.set(Some("Webhook URL is required".to_string()));
            return;
        }
        let api = api.get();
        let sid = store_id_save.clone();
        let enabled = form_enabled.get_untracked();
        set_saving.set(true);
        set_save_error.set(None);
        leptos::task::spawn_local(async move {
            let req = UpdateWebhookRequest {
                webhook_url: url.trim().to_string(),
                enabled,
            };
            match api.configure_store_webhook(&sid, &req).await {
                Ok(wh) => {
                    // Show the secret (only returned on configure)
                    set_revealed_secret.set(wh.webhook_secret);
                    set_show_form.set(false);
                    set_refresh_counter.update(|c| *c += 1);
                }
                Err(e) => {
                    set_save_error.set(Some(e.to_string()));
                }
            }
            set_saving.set(false);
        });
    };

    view! {
        <div class="store-tab-webhooks">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Webhook Configuration"</h3>
                    <p class="section-desc">"Receive real-time notifications for payment events"</p>
                </div>
            </div>

            // Secret reveal banner (shown once after configure)
            {move || revealed_secret.get().map(|secret| view! {
                <div class="detail-card" style="margin-bottom: 1rem; border: 1px solid var(--color-warning, #f59e0b);">
                    <div class="detail-card-body" style="padding: 1rem;">
                        <p style="font-weight: 500; margin-bottom: 0.5rem;">"Webhook secret (save this now — it won't be shown again):"</p>
                        <code style="word-break: break-all; font-size: var(--text-sm);">{secret}</code>
                        <button
                            class="btn btn-ghost btn-sm"
                            style="margin-top: 0.5rem;"
                            on:click=move |_| set_revealed_secret.set(None)
                        >
                            "Dismiss"
                        </button>
                    </div>
                </div>
            })}

            // Configure form
            {move || show_form.get().then(|| {
                let on_save = on_save.clone();
                view! {
                    <div class="detail-card" style="margin-bottom: 1rem;">
                        <div class="detail-card-header">
                            <h3>"Configure Webhook"</h3>
                        </div>
                        <div class="detail-card-body">
                            {move || save_error.get().map(|err| view! {
                                <div style="color: var(--color-error); margin-bottom: 1rem; padding: 0.5rem; background: var(--color-error-bg, rgba(239,68,68,0.1)); border-radius: 4px;">
                                    {err}
                                </div>
                            })}
                            <div>
                                <label class="form-label">"Webhook URL"</label>
                                <input
                                    type="url"
                                    class="form-input"
                                    placeholder="https://example.com/webhooks/payments"
                                    prop:value=move || form_url.get()
                                    on:input=move |ev| set_form_url.set(event_target_value(&ev))
                                />
                                <p style="font-size: var(--text-xs, 0.75rem); color: var(--text-muted); margin-top: 0.25rem;">
                                    "Must be HTTPS (or http://localhost for testing)"
                                </p>
                            </div>
                            <div style="margin-top: 1rem; display: flex; align-items: center; gap: 0.5rem;">
                                <input
                                    type="checkbox"
                                    prop:checked=move || form_enabled.get()
                                    on:change=move |ev| {
                                        let checked = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|e| e.checked()).unwrap_or(true);
                                        set_form_enabled.set(checked);
                                    }
                                />
                                <label class="form-label" style="margin: 0;">"Enabled"</label>
                            </div>
                            <div style="margin-top: 1rem; display: flex; gap: 0.5rem;">
                                <button
                                    class="btn btn-primary btn-sm"
                                    on:click=on_save
                                    disabled=move || saving.get()
                                >
                                    {move || if saving.get() { "Saving..." } else { "Save" }}
                                </button>
                                <button
                                    class="btn btn-secondary btn-sm"
                                    on:click=move |_| set_show_form.set(false)
                                >
                                    "Cancel"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            })}

            <Suspense fallback=move || view! {
                <div style="text-align: center; padding: 2rem; color: var(--text-muted);">
                    "Loading webhook..."
                </div>
            }>
                {move || {
                    let store_id = store_id.clone();
                    webhook_resource.get().map(move |result| match &*result {
                        Ok(Some(wh)) => {
                            let wh = wh.clone();
                            let url_for_edit = wh.webhook_url.clone();
                            let enabled_for_edit = wh.enabled;
                            let on_edit = move |_| {
                                set_form_url.set(url_for_edit.clone());
                                set_form_enabled.set(enabled_for_edit);
                                set_show_form.set(true);
                            };
                            let on_regenerate = {
                                let url = wh.webhook_url.clone();
                                let enabled = wh.enabled;
                                let store_id = store_id.clone();
                                move |_| {
                                    let api = api.get();
                                    let sid = store_id.clone();
                                    let url = url.clone();
                                    leptos::task::spawn_local(async move {
                                        let req = UpdateWebhookRequest {
                                            webhook_url: url,
                                            enabled,
                                        };
                                        if let Ok(wh) = api.configure_store_webhook(&sid, &req).await {
                                            set_revealed_secret.set(wh.webhook_secret);
                                            set_refresh_counter.update(|c| *c += 1);
                                        }
                                    });
                                }
                            };
                            let on_delete = {
                                let store_id = store_id.clone();
                                move |_| {
                                    let api = api.get();
                                    let sid = store_id.clone();
                                    leptos::task::spawn_local(async move {
                                        let _ = api.delete_store_webhook(&sid).await;
                                        set_revealed_secret.set(None);
                                        set_refresh_counter.update(|c| *c += 1);
                                    });
                                }
                            };
                            view! { <WebhookConfig webhook=wh on_edit=on_edit on_regenerate=on_regenerate on_delete=on_delete /> }.into_any()
                        }
                        Ok(None) => {
                            let on_configure = move |_| {
                                set_form_url.set(String::new());
                                set_form_enabled.set(true);
                                set_show_form.set(true);
                            };
                            view! { <WebhookEmpty on_configure=on_configure /> }.into_any()
                        }
                        Err(e) => view! {
                            <div style="text-align: center; padding: 2rem; color: var(--color-error);">
                                <p>"Failed to load webhook: "{e.to_string()}</p>
                                <button
                                    class="btn btn-secondary btn-sm"
                                    style="margin-top: 1rem;"
                                    on:click=move |_| set_refresh_counter.update(|c| *c += 1)
                                >
                                    "Retry"
                                </button>
                            </div>
                        }.into_any(),
                    })
                }}
            </Suspense>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Webhook Events"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="webhook-events">
                        <div class="webhook-event">
                            <code>"payment.detected"</code>
                            <span>"When a payment is first detected on chain"</span>
                        </div>
                        <div class="webhook-event">
                            <code>"payment.confirmed"</code>
                            <span>"When a payment reaches required confirmations"</span>
                        </div>
                        <div class="webhook-event">
                            <code>"invoice.paid"</code>
                            <span>"When an invoice is fully paid"</span>
                        </div>
                        <div class="webhook-event">
                            <code>"invoice.expired"</code>
                            <span>"When an invoice expires without payment"</span>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Webhook configuration display.
#[component]
fn WebhookConfig(
    webhook: StoreWebhook,
    on_edit: impl Fn(leptos::ev::MouseEvent) + 'static,
    on_regenerate: impl Fn(leptos::ev::MouseEvent) + 'static,
    on_delete: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    let status_class = if webhook.enabled {
        "badge badge-success"
    } else {
        "badge badge-neutral"
    };
    let status_label = if webhook.enabled {
        "Active"
    } else {
        "Disabled"
    };
    let updated_display = format_date(&webhook.updated_at);

    view! {
        <div class="detail-card">
            <div class="detail-card-header">
                <h3>"Endpoint"</h3>
                <span class=status_class>{status_label}</span>
            </div>
            <div class="detail-card-body">
                <div class="webhook-url-row">
                    <code class="webhook-url">{webhook.webhook_url}</code>
                </div>

                <div class="webhook-details">
                    <div class="webhook-detail">
                        <span class="webhook-detail-label">"Last Updated"</span>
                        <span>{updated_display}</span>
                    </div>
                </div>

                <div class="form-actions" style="display: flex; gap: 0.5rem; margin-top: 1rem;">
                    <button class="btn btn-secondary btn-sm" on:click=on_edit>"Edit endpoint"</button>
                    <button class="btn btn-secondary btn-sm" on:click=on_regenerate>"Regenerate secret"</button>
                    <button class="btn btn-ghost btn-sm" style="color: var(--color-error);" on:click=on_delete>"Delete"</button>
                </div>
            </div>
        </div>
    }
}

/// Empty webhook state.
#[component]
fn WebhookEmpty(on_configure: impl Fn(leptos::ev::MouseEvent) + 'static) -> impl IntoView {
    view! {
        <div class="detail-card">
            <div class="detail-card-body webhook-empty">
                <IconWebhook />
                <h4>"No webhook configured"</h4>
                <p>"Set up a webhook endpoint to receive real-time payment notifications"</p>
                <button class="btn btn-primary btn-sm" on:click=on_configure>"Configure webhook"</button>
            </div>
        </div>
    }
}
