//! Store management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::api::{CreateStoreRequest, EvmApiClient, Store, StorePaymentMethod, StoreWebhook, UpdateStoreRequest};

/// Helper to get chain name from chain ID.
fn chain_name(chain_id: u64) -> &'static str {
    match chain_id {
        1 => "Ethereum",
        137 => "Polygon",
        42161 => "Arbitrum",
        10 => "Optimism",
        8453 => "Base",
        56 => "BSC",
        43114 => "Avalanche",
        324 => "zkSync",
        59144 => "Linea",
        534352 => "Scroll",
        100 => "Gnosis",
        250 => "Fantom",
        11155111 => "Sepolia",
        _ => "Unknown",
    }
}

/// Format ISO date string for display.
fn format_date(iso: &str) -> String {
    if iso.len() >= 10 {
        let date_part = &iso[..10];
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() == 3 {
            let month = match parts[1] {
                "01" => "Jan", "02" => "Feb", "03" => "Mar", "04" => "Apr",
                "05" => "May", "06" => "Jun", "07" => "Jul", "08" => "Aug",
                "09" => "Sep", "10" => "Oct", "11" => "Nov", "12" => "Dec",
                _ => parts[1],
            };
            return format!("{} {}, {}", month, parts[2], parts[0]);
        }
    }
    iso.to_string()
}

/// Stores list page.
#[component]
pub fn StoresPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    // Fetch stores from API
    let (refresh_counter, set_refresh_counter) = signal(0u32);
    let stores_resource = LocalResource::new(move || {
        let api = api.get();
        refresh_counter.get(); // Track to allow manual refresh
        async move { api.list_stores().await }
    });

    // Create store form state
    let (show_create_form, set_show_create_form) = signal(false);
    let (new_store_name, set_new_store_name) = signal(String::new());
    let (creating, set_creating) = signal(false);
    let (create_error, set_create_error) = signal(None::<String>);

    let on_create_store = move |_| {
        let name = new_store_name.get_untracked();
        if name.trim().is_empty() {
            return;
        }
        let api = api.get();
        set_creating.set(true);
        set_create_error.set(None);
        leptos::task::spawn_local(async move {
            let req = CreateStoreRequest {
                name: name.trim().to_string(),
                website: None,
            };
            match api.create_store(&req).await {
                Ok(_) => {
                    set_new_store_name.set(String::new());
                    set_show_create_form.set(false);
                    set_refresh_counter.update(|c| *c += 1);
                }
                Err(e) => {
                    set_create_error.set(Some(e.to_string()));
                }
            }
            set_creating.set(false);
        });
    };

    // Filter to show only active stores by default
    let (show_archived, set_show_archived) = signal(false);

    view! {
        <div class="stores-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Stores"</h1>
                    <p class="page-description">"Manage your payment stores and configurations"</p>
                </div>
                <div class="page-actions">
                    <button
                        class="btn btn-primary btn-sm"
                        on:click=move |_| set_show_create_form.update(|v| *v = !*v)
                    >
                        <IconPlus />
                        "Create store"
                    </button>
                </div>
            </div>

            // Create store form
            {move || show_create_form.get().then(|| view! {
                <div class="detail-card" style="margin-bottom: 1.5rem;">
                    <div class="detail-card-header">
                        <h3>"New Store"</h3>
                    </div>
                    <div class="detail-card-body">
                        {move || create_error.get().map(|e| view! {
                            <div class="form-error" style="color: var(--color-error); margin-bottom: 0.75rem;">{e}</div>
                        })}
                        <div class="form-group">
                            <label class="form-label">"Store Name"</label>
                            <input
                                type="text"
                                class="form-input"
                                placeholder="My Store"
                                prop:value=move || new_store_name.get()
                                on:input=move |ev| set_new_store_name.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        ev.prevent_default();
                                        let name = new_store_name.get_untracked();
                                        if name.trim().is_empty() {
                                            return;
                                        }
                                        let api = api.get();
                                        set_creating.set(true);
                                        set_create_error.set(None);
                                        leptos::task::spawn_local(async move {
                                            let req = CreateStoreRequest {
                                                name: name.trim().to_string(),
                                                website: None,
                                            };
                                            match api.create_store(&req).await {
                                                Ok(_) => {
                                                    set_new_store_name.set(String::new());
                                                    set_show_create_form.set(false);
                                                    set_refresh_counter.update(|c| *c += 1);
                                                }
                                                Err(e) => {
                                                    set_create_error.set(Some(e.to_string()));
                                                }
                                            }
                                            set_creating.set(false);
                                        });
                                    }
                                }
                            />
                        </div>
                        <div class="form-actions">
                            <button
                                class="btn btn-primary btn-sm"
                                on:click=on_create_store.clone()
                                disabled=move || creating.get()
                            >
                                {move || if creating.get() { "Creating..." } else { "Create" }}
                            </button>
                            <button
                                class="btn btn-secondary btn-sm"
                                on:click=move |_| set_show_create_form.set(false)
                            >
                                "Cancel"
                            </button>
                        </div>
                    </div>
                </div>
            })}

            // Toolbar
            <div class="stores-toolbar">
                <label class="checkbox-label">
                    <input
                        type="checkbox"
                        prop:checked=move || show_archived.get()
                        on:change=move |ev| set_show_archived.set(event_target_checked(&ev))
                    />
                    <span>"Show archived"</span>
                </label>
            </div>

            // Stores Grid
            <Suspense fallback=move || view! {
                <div class="stores-loading" style="text-align: center; padding: 3rem; color: var(--text-muted);">
                    "Loading stores..."
                </div>
            }>
                {move || stores_resource.get().map(|result| match &*result {
                    Ok(stores) => {
                        if stores.is_empty() {
                            view! {
                                <div class="stores-empty" style="text-align: center; padding: 3rem;">
                                    <IconStore />
                                    <h3 style="margin-top: 1rem;">"No stores yet"</h3>
                                    <p style="color: var(--text-muted);">"Create your first store to start accepting payments"</p>
                                </div>
                            }.into_any()
                        } else {
                            let show = show_archived.get();
                            view! {
                                <div class="stores-grid">
                                    {stores.iter()
                                        .filter(|s| show || !s.archived)
                                        .cloned()
                                        .map(|store| view! { <StoreCard store=store /> })
                                        .collect_view()
                                    }
                                </div>
                            }.into_any()
                        }
                    }
                    Err(e) => view! {
                        <div class="stores-error" style="text-align: center; padding: 3rem; color: var(--color-error);">
                            <p>"Failed to load stores: "{e.to_string()}</p>
                            <button
                                class="btn btn-secondary btn-sm"
                                style="margin-top: 1rem;"
                                on:click=move |_| set_refresh_counter.update(|c| *c += 1)
                            >
                                "Retry"
                            </button>
                        </div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

/// Store card component.
#[component]
fn StoreCard(store: Store) -> impl IntoView {
    let store_id = store.id.clone();
    let store_link = store.id.clone();
    let store_name = store.name.clone();
    let created_display = format_date(&store.created_at);

    view! {
        <A href=format!("/evm/stores/{}", store_link) attr:class="store-card">
            <div class="store-card-header">
                <div class="store-card-icon">
                    <IconStore />
                </div>
                <div class="store-card-title">
                    <h3 class="store-card-name">{store_name}</h3>
                    {store.archived.then(|| view! {
                        <span class="badge badge-neutral">"Archived"</span>
                    })}
                </div>
            </div>

            <div class="store-card-body">
                {store.website.clone().map(|url| view! {
                    <div class="store-card-row">
                        <IconGlobe />
                        <span class="store-card-website">{url}</span>
                    </div>
                })}
                <div class="store-card-row">
                    <IconCalendar />
                    <span class="store-card-date">"Created "{created_display}</span>
                </div>
            </div>

            <div class="store-card-footer">
                <span class="store-card-id">{store_id}</span>
                <IconChevronRight />
            </div>
        </A>
    }
}

/// Store detail page.
#[component]
pub fn StoreDetailPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let params = use_params_map();
    let store_id = move || params.get().get("id").unwrap_or_default();

    // Active tab state
    let (active_tab, set_active_tab) = signal("general".to_string());

    // Fetch store from API
    let store_resource = LocalResource::new(move || {
        let api = api.get();
        let id = store_id();
        async move { api.get_store(&id).await }
    });

    let tabs = vec![
        ("general", "General"),
        ("payment_methods", "Payment Methods"),
        ("webhooks", "Webhooks"),
    ];

    view! {
        <div class="store-detail-page">
            <Suspense fallback=move || view! {
                <div style="text-align: center; padding: 3rem; color: var(--text-muted);">
                    "Loading store..."
                </div>
            }>
                {move || store_resource.get().map(|result| match &*result {
                    Ok(store) => {
                        let store_name = store.name.clone();
                        let created_display = format_date(&store.created_at);
                        let store_for_tabs = store.clone();

                        view! {
                            // Header
                            <div class="store-detail-header">
                                <div class="store-detail-header-left">
                                    <A href="/evm/stores" attr:class="back-link">
                                        <IconArrowLeft />
                                        "Stores"
                                    </A>
                                    <div class="store-detail-title-row">
                                        <h1 class="store-detail-title">{store_name}</h1>
                                        {store.archived.then(|| view! {
                                            <span class="badge badge-neutral">"Archived"</span>
                                        })}
                                    </div>
                                    <p class="store-detail-subtitle">
                                        <code class="store-detail-id">{store.id.clone()}</code>
                                        " · Created "{created_display}
                                    </p>
                                </div>
                                <div class="store-detail-actions">
                                    {(!store.archived).then(|| view! {
                                        <button class="btn btn-secondary btn-sm">
                                            <IconArchive />
                                            "Archive"
                                        </button>
                                    })}
                                </div>
                            </div>

                            // Tabs
                            <div class="store-tabs">
                                {tabs.clone().into_iter().map(|(key, label)| {
                                    let key_owned = key.to_string();
                                    let key_for_click = key.to_string();
                                    view! {
                                        <button
                                            class=move || if active_tab.get() == key_owned { "store-tab active" } else { "store-tab" }
                                            on:click=move |_| set_active_tab.set(key_for_click.clone())
                                        >
                                            {label}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>

                            // Tab Content
                            <div class="store-tab-content">
                                {move || match active_tab.get().as_str() {
                                    "general" => view! { <GeneralTab store=store_for_tabs.clone() /> }.into_any(),
                                    "payment_methods" => view! { <PaymentMethodsTab store_id=store_for_tabs.id.clone() /> }.into_any(),
                                    "webhooks" => view! { <WebhooksTab store_id=store_for_tabs.id.clone() /> }.into_any(),
                                    _ => view! { <GeneralTab store=store_for_tabs.clone() /> }.into_any(),
                                }}
                            </div>
                        }.into_any()
                    }
                    Err(e) => view! {
                        <div style="text-align: center; padding: 3rem;">
                            <A href="/evm/stores" attr:class="back-link">
                                <IconArrowLeft />
                                "Back to Stores"
                            </A>
                            <p style="color: var(--color-error); margin-top: 1rem;">
                                "Failed to load store: "{e.to_string()}
                            </p>
                        </div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

/// General settings tab.
#[component]
fn GeneralTab(store: Store) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let navigate = use_navigate();
    let store_id = store.id.clone();

    let (name, set_name) = signal(store.name.clone());
    let (website, set_website) = signal(store.website.clone().unwrap_or_default());
    let (saving, set_saving) = signal(false);
    let (save_message, set_save_message) = signal(None::<(bool, String)>); // (is_success, message)
    let (deleting, set_deleting) = signal(false);

    let store_id_save = store_id.clone();
    let on_save = move |_| {
        let api = api.get();
        let id = store_id_save.clone();
        let new_name = name.get_untracked();
        let new_website = website.get_untracked();
        set_saving.set(true);
        set_save_message.set(None);
        leptos::task::spawn_local(async move {
            let req = UpdateStoreRequest {
                name: Some(new_name),
                website: Some(if new_website.is_empty() { String::new() } else { new_website }),
            };
            match api.update_store(&id, &req).await {
                Ok(_) => set_save_message.set(Some((true, "Changes saved".to_string()))),
                Err(e) => set_save_message.set(Some((false, e.to_string()))),
            }
            set_saving.set(false);
        });
    };

    let store_id_delete = store_id.clone();
    let on_delete = move |_| {
        let api = api.get();
        let id = store_id_delete.clone();
        let navigate = navigate.clone();
        set_deleting.set(true);
        leptos::task::spawn_local(async move {
            match api.delete_store(&id).await {
                Ok(_) => navigate("/evm/stores", Default::default()),
                Err(e) => {
                    web_sys::window()
                        .and_then(|w| w.alert_with_message(&format!("Delete failed: {}", e)).ok());
                }
            }
            set_deleting.set(false);
        });
    };

    view! {
        <div class="store-tab-general">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Store Information"</h3>
                </div>
                <div class="detail-card-body">
                    {move || save_message.get().map(|(success, msg)| {
                        let class = if success { "color: var(--color-success)" } else { "color: var(--color-error)" };
                        view! { <p style=class>{msg}</p> }
                    })}

                    <div class="form-group">
                        <label class="form-label">"Store Name"</label>
                        <input
                            type="text"
                            class="form-input"
                            prop:value=move || name.get()
                            on:input=move |ev| set_name.set(event_target_value(&ev))
                        />
                        <p class="form-help">"The name displayed to your customers"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Website"</label>
                        <input
                            type="url"
                            class="form-input"
                            placeholder="https://example.com"
                            prop:value=move || website.get()
                            on:input=move |ev| set_website.set(event_target_value(&ev))
                        />
                        <p class="form-help">"Your store's website URL (optional)"</p>
                    </div>

                    <div class="form-actions">
                        <button
                            class="btn btn-primary btn-sm"
                            on:click=on_save
                            disabled=move || saving.get()
                        >
                            {move || if saving.get() { "Saving..." } else { "Save changes" }}
                        </button>
                    </div>
                </div>
            </div>

            <div class="detail-card detail-card-danger">
                <div class="detail-card-header">
                    <h3>"Danger Zone"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="danger-action">
                        <div class="danger-action-info">
                            <span class="danger-action-title">"Delete this store"</span>
                            <span class="danger-action-desc">"Permanently delete this store and all its data"</span>
                        </div>
                        <button
                            class="btn btn-danger btn-sm"
                            on:click=on_delete
                            disabled=move || deleting.get()
                        >
                            {move || if deleting.get() { "Deleting..." } else { "Delete store" }}
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Payment methods tab.
#[component]
fn PaymentMethodsTab(#[allow(unused)] store_id: String) -> impl IntoView {
    // TODO: Fetch from API when payment methods client methods are added
    let methods: Vec<StorePaymentMethod> = vec![];
    let enabled_count = methods.iter().filter(|m| m.enabled).count();
    let total_count = methods.len();

    view! {
        <div class="store-tab-payment-methods">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Payment Methods"</h3>
                    <p class="section-desc">{enabled_count}" of "{total_count}" methods enabled"</p>
                </div>
                <button class="btn btn-primary btn-sm">
                    <IconPlus />
                    "Add method"
                </button>
            </div>

            <div class="payment-methods-table-container">
                <table class="payment-methods-table">
                    <thead>
                        <tr>
                            <th>"Asset"</th>
                            <th>"Network"</th>
                            <th>"Type"</th>
                            <th>"Derivation Index"</th>
                            <th>"Status"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        {methods.into_iter().map(|method| {
                            view! { <PaymentMethodRow method=method /> }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </div>
    }
}

/// Payment method table row.
#[component]
fn PaymentMethodRow(method: StorePaymentMethod) -> impl IntoView {
    let network = chain_name(method.chain_id);
    let asset_type = if method.token_address.is_some() { "ERC20" } else { "Native" };
    let status_class = if method.enabled { "badge badge-success" } else { "badge badge-neutral" };
    let status_label = if method.enabled { "Enabled" } else { "Disabled" };

    view! {
        <tr class="payment-method-row">
            <td>
                <div class="payment-method-asset">
                    <span class="payment-method-symbol">{method.asset_symbol}</span>
                </div>
            </td>
            <td>
                <span class="payment-method-network">{network}</span>
            </td>
            <td>
                <span class="payment-method-type">{asset_type}</span>
            </td>
            <td>
                <code class="payment-method-index">{method.derivation_index}</code>
            </td>
            <td>
                <span class=status_class>{status_label}</span>
            </td>
            <td>
                <button class="btn btn-ghost btn-sm btn-icon">
                    <IconMore />
                </button>
            </td>
        </tr>
    }
}

/// Webhooks tab.
#[component]
fn WebhooksTab(#[allow(unused)] store_id: String) -> impl IntoView {
    // TODO: Fetch from API when webhook client methods are added
    let webhook: Option<StoreWebhook> = None;
    view! {
        <div class="store-tab-webhooks">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Webhook Configuration"</h3>
                    <p class="section-desc">"Receive real-time notifications for payment events"</p>
                </div>
            </div>

            {match webhook {
                Some(wh) => view! { <WebhookConfig webhook=wh /> }.into_any(),
                None => view! { <WebhookEmpty /> }.into_any(),
            }}

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
fn WebhookConfig(webhook: StoreWebhook) -> impl IntoView {
    let status_class = if webhook.enabled { "badge badge-success" } else { "badge badge-neutral" };
    let status_label = if webhook.enabled { "Active" } else { "Disabled" };
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
                    <button class="btn btn-ghost btn-sm btn-icon" title="Copy URL">
                        <IconCopy />
                    </button>
                </div>

                <div class="webhook-details">
                    <div class="webhook-detail">
                        <span class="webhook-detail-label">"Signing Secret"</span>
                        <div class="webhook-secret-row">
                            <code class="webhook-secret">{webhook.webhook_secret}</code>
                            <button class="btn btn-ghost btn-sm btn-icon" title="Reveal">
                                <IconEye />
                            </button>
                        </div>
                    </div>
                    <div class="webhook-detail">
                        <span class="webhook-detail-label">"Last Updated"</span>
                        <span>{updated_display}</span>
                    </div>
                </div>

                <div class="form-actions">
                    <button class="btn btn-secondary btn-sm">"Edit endpoint"</button>
                    <button class="btn btn-secondary btn-sm">"Regenerate secret"</button>
                    <button class="btn btn-ghost btn-sm">"Send test event"</button>
                </div>
            </div>
        </div>
    }
}

/// Empty webhook state.
#[component]
fn WebhookEmpty() -> impl IntoView {
    view! {
        <div class="detail-card">
            <div class="detail-card-body webhook-empty">
                <IconWebhook />
                <h4>"No webhook configured"</h4>
                <p>"Set up a webhook endpoint to receive real-time payment notifications"</p>
                <button class="btn btn-primary btn-sm">"Configure webhook"</button>
            </div>
        </div>
    }
}

// ============================================
// Icons
// ============================================

#[component]
fn IconPlus() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}

#[component]
fn IconStore() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path>
            <polyline points="9 22 9 12 15 12 15 22"></polyline>
        </svg>
    }
}

#[component]
fn IconGlobe() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10"></circle>
            <line x1="2" y1="12" x2="22" y2="12"></line>
            <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path>
        </svg>
    }
}

#[component]
fn IconCalendar() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="18" rx="2" ry="2"></rect>
            <line x1="16" y1="2" x2="16" y2="6"></line>
            <line x1="8" y1="2" x2="8" y2="6"></line>
            <line x1="3" y1="10" x2="21" y2="10"></line>
        </svg>
    }
}

#[component]
fn IconChevronRight() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
    }
}

#[component]
fn IconArrowLeft() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="19" y1="12" x2="5" y2="12"></line>
            <polyline points="12 19 5 12 12 5"></polyline>
        </svg>
    }
}

#[component]
fn IconArchive() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="21 8 21 21 3 21 3 8"></polyline>
            <rect x="1" y="3" width="22" height="5"></rect>
            <line x1="10" y1="12" x2="14" y2="12"></line>
        </svg>
    }
}

#[component]
fn IconMore() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="1"></circle>
            <circle cx="19" cy="12" r="1"></circle>
            <circle cx="5" cy="12" r="1"></circle>
        </svg>
    }
}

#[component]
fn IconCopy() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
        </svg>
    }
}

#[component]
fn IconEye() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"></path>
            <circle cx="12" cy="12" r="3"></circle>
        </svg>
    }
}

#[component]
fn IconWebhook() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 16.98h-5.99c-1.1 0-1.95.94-2.48 1.9A4 4 0 0 1 2 17c.01-.7.2-1.4.57-2"></path>
            <path d="m6 17 3.13-5.78c.53-.97.1-2.18-.5-3.1a4 4 0 1 1 6.89-4.06"></path>
            <path d="m12 6 3.13 5.73C15.66 12.7 16.9 13 18 13a4 4 0 0 1 0 8"></path>
        </svg>
    }
}
