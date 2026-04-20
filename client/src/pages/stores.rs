//! Store management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};
use wasm_bindgen::JsCast;

use crate::api::{
    CreatePaymentMethodRequest, CreateStoreRequest, EvmApiClient, Store, StorePaymentMethod,
    StoreSettings, StoreWebhook, UpdatePaymentMethodRequest, UpdateStoreRequest,
    UpdateStoreSettingsRequest, UpdateWebhookRequest,
};
use crate::app::StoreContext;

use crate::util::chain_name;

/// Format ISO date string for display.
fn format_date(iso: &str) -> String {
    if iso.len() >= 10 {
        let date_part = &iso[..10];
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() == 3 {
            let month = match parts[1] {
                "01" => "Jan",
                "02" => "Feb",
                "03" => "Mar",
                "04" => "Apr",
                "05" => "May",
                "06" => "Jun",
                "07" => "Jul",
                "08" => "Aug",
                "09" => "Sep",
                "10" => "Oct",
                "11" => "Nov",
                "12" => "Dec",
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
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

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

    let on_create_store = {
        let store_ctx = store_ctx.clone();
        move |_| {
            let name = new_store_name.get_untracked();
            if name.trim().is_empty() {
                return;
            }
            let api = api.get();
            let store_ctx = store_ctx.clone();
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
                        store_ctx.refetch_stores();
                    }
                    Err(e) => {
                        set_create_error.set(Some(e.to_string()));
                    }
                }
                set_creating.set(false);
            });
        }
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
                                on:keydown={
                                    let store_ctx = store_ctx.clone();
                                    move |ev: web_sys::KeyboardEvent| {
                                        if ev.key() == "Enter" {
                                            ev.prevent_default();
                                            let name = new_store_name.get_untracked();
                                            if name.trim().is_empty() {
                                                return;
                                            }
                                            let api = api.get();
                                            let store_ctx = store_ctx.clone();
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
                                                        store_ctx.refetch_stores();
                                                    }
                                                    Err(e) => {
                                                        set_create_error.set(Some(e.to_string()));
                                                    }
                                                }
                                                set_creating.set(false);
                                            });
                                        }
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
        ("settings", "Settings"),
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
                                    "settings" => view! { <SettingsTab store_id=store_for_tabs.id.clone() /> }.into_any(),
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
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");
    let navigate = use_navigate();
    let store_id = store.id.clone();

    let (name, set_name) = signal(store.name.clone());
    let (website, set_website) = signal(store.website.clone().unwrap_or_default());
    let (saving, set_saving) = signal(false);
    let (save_message, set_save_message) = signal(None::<(bool, String)>); // (is_success, message)
    let (deleting, set_deleting) = signal(false);

    let store_id_save = store_id.clone();
    let on_save = {
        let store_ctx = store_ctx.clone();
        move |_| {
            let api = api.get();
            let id = store_id_save.clone();
            let new_name = name.get_untracked();
            let new_website = website.get_untracked();
            let store_ctx = store_ctx.clone();
            set_saving.set(true);
            set_save_message.set(None);
            leptos::task::spawn_local(async move {
                let req = UpdateStoreRequest {
                    name: Some(new_name),
                    website: Some(if new_website.is_empty() {
                        String::new()
                    } else {
                        new_website
                    }),
                };
                match api.update_store(&id, &req).await {
                    Ok(_) => {
                        set_save_message.set(Some((true, "Changes saved".to_string())));
                        store_ctx.refetch_stores();
                    }
                    Err(e) => set_save_message.set(Some((false, e.to_string()))),
                }
                set_saving.set(false);
            });
        }
    };

    let store_id_delete = store_id.clone();
    let on_delete = move |_| {
        let api = api.get();
        let id = store_id_delete.clone();
        let navigate = navigate.clone();
        let store_ctx = store_ctx.clone();
        set_deleting.set(true);
        leptos::task::spawn_local(async move {
            match api.delete_store(&id).await {
                Ok(_) => {
                    store_ctx.refetch_stores();
                    navigate("/evm/stores", Default::default());
                }
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
fn PaymentMethodsTab(store_id: String) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    // Fetch payment methods from API
    let (refresh_counter, set_refresh_counter) = signal(0u32);
    let store_id_fetch = store_id.clone();
    let methods_resource = LocalResource::new(move || {
        let api = api.get();
        let id = store_id_fetch.clone();
        refresh_counter.get();
        async move { api.list_payment_methods(&id).await }
    });

    // Create form state
    let (show_create_form, set_show_create_form) = signal(false);
    let (new_chain_id, set_new_chain_id) = signal("11155111".to_string());
    let (new_asset_symbol, set_new_asset_symbol) = signal(String::new());
    let (new_token_address, set_new_token_address) = signal(String::new());
    let (new_decimals, set_new_decimals) = signal("18".to_string());
    let (new_xpub, set_new_xpub) = signal(String::new());
    let (creating, set_creating) = signal(false);
    let (create_error, set_create_error) = signal(None::<String>);

    let store_id_create = store_id.clone();
    let on_create = move |_| {
        let xpub = new_xpub.get_untracked();
        let symbol = new_asset_symbol.get_untracked();
        if xpub.trim().is_empty() || symbol.trim().is_empty() {
            set_create_error.set(Some("Asset symbol and xpub are required".to_string()));
            return;
        }
        let chain_id: u64 = match new_chain_id.get_untracked().parse() {
            Ok(v) => v,
            Err(_) => {
                set_create_error.set(Some("Invalid chain ID".to_string()));
                return;
            }
        };
        let decimals: u8 = match new_decimals.get_untracked().parse() {
            Ok(v) => v,
            Err(_) => {
                set_create_error.set(Some("Invalid decimals".to_string()));
                return;
            }
        };
        let token_address = {
            let addr = new_token_address.get_untracked();
            if addr.trim().is_empty() {
                None
            } else {
                Some(addr.trim().to_string())
            }
        };
        let api = api.get();
        let sid = store_id_create.clone();
        set_creating.set(true);
        set_create_error.set(None);
        leptos::task::spawn_local(async move {
            let req = CreatePaymentMethodRequest {
                chain_id,
                token_address,
                asset_symbol: symbol.trim().to_string(),
                decimals,
                xpub: xpub.trim().to_string(),
            };
            match api.create_payment_method(&sid, &req).await {
                Ok(_) => {
                    set_show_create_form.set(false);
                    set_new_asset_symbol.set(String::new());
                    set_new_token_address.set(String::new());
                    set_new_xpub.set(String::new());
                    set_new_decimals.set("18".to_string());
                    set_refresh_counter.update(|c| *c += 1);
                }
                Err(e) => {
                    set_create_error.set(Some(e.to_string()));
                }
            }
            set_creating.set(false);
        });
    };

    view! {
        <div class="store-tab-payment-methods">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Payment Methods"</h3>
                    <Suspense fallback=|| view! { <p class="section-desc">"Loading..."</p> }>
                        {move || methods_resource.get().map(|result| match &*result {
                            Ok(methods) => {
                                let enabled = methods.iter().filter(|m| m.enabled).count();
                                let total = methods.len();
                                view! { <p class="section-desc">{enabled}" of "{total}" methods enabled"</p> }.into_any()
                            }
                            Err(_) => view! { <p class="section-desc">""</p> }.into_any(),
                        })}
                    </Suspense>
                </div>
                <button
                    class="btn btn-primary btn-sm"
                    on:click=move |_| set_show_create_form.update(|v| *v = !*v)
                >
                    <IconPlus />
                    "Add method"
                </button>
            </div>

            // Create form
            {move || show_create_form.get().then(|| {
                let on_create = on_create.clone();
                view! {
                    <div class="detail-card" style="margin-bottom: 1.5rem;">
                        <div class="detail-card-header">
                            <h3>"Add Payment Method"</h3>
                        </div>
                        <div class="detail-card-body">
                            {move || create_error.get().map(|err| view! {
                                <div class="form-error" style="color: var(--color-error); margin-bottom: 1rem; padding: 0.5rem; background: var(--color-error-bg, rgba(239,68,68,0.1)); border-radius: 4px;">
                                    {err}
                                </div>
                            })}
                            <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 1rem;">
                                <div>
                                    <label class="form-label">"Network (Chain ID)"</label>
                                    <select
                                        class="form-select"
                                        on:change=move |ev| set_new_chain_id.set(event_target_value(&ev))
                                        prop:value=move || new_chain_id.get()
                                    >
                                        <option value="1">"Ethereum (1)"</option>
                                        <option value="137">"Polygon (137)"</option>
                                        <option value="42161">"Arbitrum (42161)"</option>
                                        <option value="10">"Optimism (10)"</option>
                                        <option value="8453">"Base (8453)"</option>
                                        <option value="56">"BSC (56)"</option>
                                        <option value="43114">"Avalanche (43114)"</option>
                                        <option value="324">"zkSync (324)"</option>
                                        <option value="59144">"Linea (59144)"</option>
                                        <option value="534352">"Scroll (534352)"</option>
                                        <option value="100">"Gnosis (100)"</option>
                                        <option value="250">"Fantom (250)"</option>
                                        <option value="11155111" selected>"Sepolia (11155111)"</option>
                                    </select>
                                </div>
                                <div>
                                    <label class="form-label">"Asset Symbol"</label>
                                    <input
                                        type="text"
                                        class="form-input"
                                        placeholder="ETH"
                                        prop:value=move || new_asset_symbol.get()
                                        on:input=move |ev| set_new_asset_symbol.set(event_target_value(&ev))
                                    />
                                </div>
                                <div>
                                    <label class="form-label">"Token Address (ERC20 only)"</label>
                                    <input
                                        type="text"
                                        class="form-input"
                                        placeholder="0x... (leave empty for native asset)"
                                        prop:value=move || new_token_address.get()
                                        on:input=move |ev| set_new_token_address.set(event_target_value(&ev))
                                    />
                                </div>
                                <div>
                                    <label class="form-label">"Decimals"</label>
                                    <input
                                        type="number"
                                        class="form-input"
                                        prop:value=move || new_decimals.get()
                                        on:input=move |ev| set_new_decimals.set(event_target_value(&ev))
                                    />
                                </div>
                            </div>
                            <div class="form-group" style="margin-top: 1rem;">
                                <label class="form-label">"Extended Public Key (xpub)"</label>
                                <input
                                    type="text"
                                    class="form-input"
                                    placeholder="xpub..."
                                    prop:value=move || new_xpub.get()
                                    on:input=move |ev| set_new_xpub.set(event_target_value(&ev))
                                />
                            </div>
                            <div class="form-actions" style="margin-top: 1rem; display: flex; gap: 0.5rem;">
                                <button
                                    class="btn btn-primary btn-sm"
                                    on:click=on_create
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
                }
            })}

            <Suspense fallback=move || view! {
                <div style="text-align: center; padding: 2rem; color: var(--text-muted);">
                    "Loading payment methods..."
                </div>
            }>
                {move || {
                    let store_id = store_id.clone();
                    methods_resource.get().map(move |result| match &*result {
                        Ok(methods) if methods.is_empty() => view! {
                            <div class="empty-state" style="text-align: center; padding: 3rem; color: var(--text-muted);">
                                <p style="font-size: 1.1rem;">"No payment methods configured"</p>
                                <p style="margin-top: 0.5rem;">"Add a payment method to start accepting payments"</p>
                            </div>
                        }.into_any(),
                        Ok(methods) => view! {
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
                                        {methods.iter().map(|method| {
                                            let method = method.clone();
                                            let sid = store_id.clone();
                                            view! { <PaymentMethodRow method=method store_id=sid set_refresh_counter=set_refresh_counter /> }
                                        }).collect_view()}
                                    </tbody>
                                </table>
                            </div>
                        }.into_any(),
                        Err(e) => view! {
                            <div style="text-align: center; padding: 2rem; color: var(--color-error);">
                                <p>"Failed to load payment methods: "{e.to_string()}</p>
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
        </div>
    }
}

/// Payment method table row.
#[component]
fn PaymentMethodRow(
    method: StorePaymentMethod,
    store_id: String,
    set_refresh_counter: WriteSignal<u32>,
) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let network = chain_name(method.chain_id);
    let asset_type = if method.token_address.is_some() {
        "ERC20"
    } else {
        "Native"
    };
    let method_id = method.id.clone();
    let method_id_toggle = method.id.clone();
    let is_enabled = method.enabled;

    let (toggling, set_toggling) = signal(false);
    let (deleting, set_deleting) = signal(false);

    let on_toggle = {
        let store_id = store_id.clone();
        move |_| {
            let api = api.get();
            let sid = store_id.clone();
            let mid = method_id_toggle.clone();
            set_toggling.set(true);
            leptos::task::spawn_local(async move {
                let req = UpdatePaymentMethodRequest {
                    enabled: Some(!is_enabled),
                    xpub: None,
                };
                let _ = api.update_payment_method(&sid, &mid, &req).await;
                set_toggling.set(false);
                set_refresh_counter.update(|c| *c += 1);
            });
        }
    };

    let on_delete = {
        let store_id = store_id.clone();
        move |_| {
            let api = api.get();
            let sid = store_id.clone();
            let mid = method_id.clone();
            set_deleting.set(true);
            leptos::task::spawn_local(async move {
                let _ = api.delete_payment_method(&sid, &mid).await;
                set_deleting.set(false);
                set_refresh_counter.update(|c| *c += 1);
            });
        }
    };

    let status_class = if method.enabled {
        "badge badge-success"
    } else {
        "badge badge-neutral"
    };
    let status_label = if method.enabled {
        "Enabled"
    } else {
        "Disabled"
    };

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
                <button
                    class=status_class
                    style="cursor: pointer; border: none; font: inherit;"
                    on:click=on_toggle
                    disabled=move || toggling.get()
                >
                    {move || if toggling.get() { "..." } else { status_label }}
                </button>
            </td>
            <td>
                <button
                    class="btn btn-ghost btn-sm"
                    style="color: var(--color-error);"
                    on:click=on_delete
                    disabled=move || deleting.get()
                >
                    {move || if deleting.get() { "..." } else { "Delete" }}
                </button>
            </td>
        </tr>
    }
}

/// Webhooks tab.
#[component]
fn WebhooksTab(store_id: String) -> impl IntoView {
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

// ============================================
// ============================================
// Settings Tab
// ============================================

/// Store settings tab: defaults, branding, notifications.
#[component]
fn SettingsTab(store_id: String) -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_id_for_load = store_id.clone();
    let store_id_for_save = store_id.clone();

    let (refresh, set_refresh) = signal(0u32);
    let settings_resource = LocalResource::new(move || {
        let api = api.get();
        let id = store_id_for_load.clone();
        refresh.get();
        async move { api.get_store_settings(&id).await }
    });

    // Form state
    let (chain_id, set_chain_id) = signal(String::new());
    let (display_currency, set_display_currency) = signal(String::new());
    let (logo_url, set_logo_url) = signal(String::new());
    let (accent_color, set_accent_color) = signal(String::new());
    let (saving, set_saving) = signal(false);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (success_msg, set_success_msg) = signal(false);

    // Notification prefs state (one bool per event)
    let (wh_payment_detected, set_wh_payment_detected) = signal(true);
    let (wh_payment_confirmed, set_wh_payment_confirmed) = signal(true);
    let (wh_invoice_expired, set_wh_invoice_expired) = signal(true);
    let (wh_invoice_cancelled, set_wh_invoice_cancelled) = signal(true);
    let (wh_late_paid, set_wh_late_paid) = signal(true);

    // Populate form when settings load
    let populate = move |s: &StoreSettings| {
        set_chain_id.set(
            s.default_chain_id
                .map(|c| c.to_string())
                .unwrap_or_default(),
        );
        set_display_currency.set(s.default_display_currency.clone().unwrap_or_default());
        set_logo_url.set(s.logo_url.clone().unwrap_or_default());
        set_accent_color.set(s.accent_color.clone().unwrap_or_default());
        // Parse notification prefs
        let prefs = &s.notification_prefs;
        let get_wh = |key: &str| -> bool {
            prefs
                .get(key)
                .and_then(|v| v.get("webhook"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
        };
        set_wh_payment_detected.set(get_wh("payment_detected"));
        set_wh_payment_confirmed.set(get_wh("payment_confirmed"));
        set_wh_invoice_expired.set(get_wh("invoice_expired"));
        set_wh_invoice_cancelled.set(get_wh("invoice_cancelled"));
        set_wh_late_paid.set(get_wh("late_paid"));
    };

    let on_save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_saving.set(true);
        set_error_msg.set(None);
        set_success_msg.set(false);

        let api = api.get();
        let id = store_id_for_save.clone();

        let chain = chain_id.get();
        let currency = display_currency.get();
        let logo = logo_url.get();
        let color = accent_color.get();

        // Build notification prefs JSON
        let prefs = serde_json::json!({
            "payment_detected": {"webhook": wh_payment_detected.get()},
            "payment_confirmed": {"webhook": wh_payment_confirmed.get()},
            "invoice_expired": {"webhook": wh_invoice_expired.get()},
            "invoice_cancelled": {"webhook": wh_invoice_cancelled.get()},
            "late_paid": {"webhook": wh_late_paid.get()},
        });

        let req = UpdateStoreSettingsRequest {
            default_chain_id: if chain.is_empty() {
                None
            } else {
                chain.parse().ok()
            },
            default_display_currency: if currency.is_empty() {
                None
            } else {
                Some(currency)
            },
            logo_url: if logo.is_empty() { None } else { Some(logo) },
            accent_color: if color.is_empty() { None } else { Some(color) },
            notification_prefs: Some(prefs),
        };

        leptos::task::spawn_local(async move {
            match api.update_store_settings(&id, &req).await {
                Ok(_) => {
                    set_success_msg.set(true);
                    set_refresh.update(|n| *n += 1);
                }
                Err(e) => set_error_msg.set(Some(format!("{}", e))),
            }
            set_saving.set(false);
        });
    };

    view! {
        <div class="settings-tab">
            <Suspense fallback=move || view! { <p style="color: var(--text-muted);">"Loading settings..."</p> }>
                {move || settings_resource.get().map(|result| match &*result {
                    Ok(settings) => {
                        populate(settings);
                        view! { <div></div> }.into_any()
                    }
                    Err(_) => view! { <div></div> }.into_any(),
                })}
            </Suspense>

            <form on:submit=on_save>
                // General section
                <div class="settings-section">
                    <h3 class="settings-section-title">"General"</h3>
                    <div class="form-group">
                        <label for="chain_id">"Default Chain"</label>
                        <select id="chain_id"
                            prop:value=move || chain_id.get()
                            on:change=move |ev| set_chain_id.set(event_target_value(&ev))
                        >
                            <option value="">"No default"</option>
                            <option value="1">"Ethereum"</option>
                            <option value="137">"Polygon"</option>
                            <option value="42161">"Arbitrum"</option>
                            <option value="10">"Optimism"</option>
                            <option value="8453">"Base"</option>
                            <option value="56">"BSC"</option>
                            <option value="43114">"Avalanche"</option>
                            <option value="100">"Gnosis"</option>
                            <option value="250">"Fantom"</option>
                        </select>
                    </div>
                    <div class="form-group">
                        <label for="display_currency">"Default Display Currency"</label>
                        <select id="display_currency"
                            prop:value=move || display_currency.get()
                            on:change=move |ev| set_display_currency.set(event_target_value(&ev))
                        >
                            <option value="">"No default"</option>
                            <option value="USD">"USD"</option>
                            <option value="EUR">"EUR"</option>
                            <option value="GBP">"GBP"</option>
                            <option value="BRL">"BRL"</option>
                            <option value="JPY">"JPY"</option>
                        </select>
                    </div>
                </div>

                // Branding section
                <div class="settings-section">
                    <h3 class="settings-section-title">"Branding"</h3>
                    <div class="form-group">
                        <label for="logo_url">"Logo URL"</label>
                        <input type="url" id="logo_url" placeholder="https://example.com/logo.png"
                            prop:value=move || logo_url.get()
                            on:input=move |ev| set_logo_url.set(event_target_value(&ev))
                        />
                        <small class="form-hint">"Must be HTTPS. Displayed on checkout page."</small>
                    </div>
                    <div class="form-group">
                        <label for="accent_color">"Accent Color"</label>
                        <div class="color-input-row">
                            <input type="color"
                                prop:value=move || {
                                    let c = accent_color.get();
                                    if c.is_empty() { "#6366f1".to_string() } else { c }
                                }
                                on:input=move |ev| set_accent_color.set(event_target_value(&ev))
                            />
                            <input type="text" id="accent_color" placeholder="#6366f1" maxlength="7"
                                prop:value=move || accent_color.get()
                                on:input=move |ev| set_accent_color.set(event_target_value(&ev))
                            />
                        </div>
                        <small class="form-hint">"Hex color for checkout button and header accent."</small>
                    </div>
                    {move || {
                        let logo = logo_url.get();
                        let color = accent_color.get();
                        let color_val = if color.is_empty() { "#6366f1".to_string() } else { color };
                        if !logo.is_empty() || !accent_color.get().is_empty() {
                            view! {
                                <div class="branding-preview" style=format!("border-top: 3px solid {};", color_val)>
                                    {(!logo.is_empty()).then(|| view! {
                                        <img src=logo alt="Logo preview" style="max-height: 40px; margin-bottom: 0.5rem;" />
                                    })}
                                    <span style="font-size: 0.875rem; color: var(--text-muted);">"Checkout header preview"</span>
                                </div>
                            }.into_any()
                        } else {
                            view! { <div></div> }.into_any()
                        }
                    }}
                </div>

                // Notifications section
                <div class="settings-section">
                    <h3 class="settings-section-title">"Notifications"</h3>
                    <p class="settings-section-desc">"Control which events trigger webhook delivery."</p>
                    <div class="notification-matrix">
                        <div class="notification-row">
                            <label>"Payment Detected"</label>
                            <input type="checkbox"
                                prop:checked=move || wh_payment_detected.get()
                                on:change=move |ev| set_wh_payment_detected.set(event_target_checked(&ev))
                            />
                        </div>
                        <div class="notification-row">
                            <label>"Payment Confirmed"</label>
                            <input type="checkbox"
                                prop:checked=move || wh_payment_confirmed.get()
                                on:change=move |ev| set_wh_payment_confirmed.set(event_target_checked(&ev))
                            />
                        </div>
                        <div class="notification-row">
                            <label>"Invoice Expired"</label>
                            <input type="checkbox"
                                prop:checked=move || wh_invoice_expired.get()
                                on:change=move |ev| set_wh_invoice_expired.set(event_target_checked(&ev))
                            />
                        </div>
                        <div class="notification-row">
                            <label>"Invoice Cancelled"</label>
                            <input type="checkbox"
                                prop:checked=move || wh_invoice_cancelled.get()
                                on:change=move |ev| set_wh_invoice_cancelled.set(event_target_checked(&ev))
                            />
                        </div>
                        <div class="notification-row">
                            <label>"Late Payment"</label>
                            <input type="checkbox"
                                prop:checked=move || wh_late_paid.get()
                                on:change=move |ev| set_wh_late_paid.set(event_target_checked(&ev))
                            />
                        </div>
                    </div>
                </div>

                // Error / success messages
                {move || error_msg.get().map(|msg| view! {
                    <div class="alert alert-error">{msg}</div>
                })}
                {move || success_msg.get().then(|| view! {
                    <div class="alert alert-success">"Settings saved."</div>
                })}

                <div class="form-actions">
                    <button type="submit" class="btn btn-primary" disabled=move || saving.get()>
                        {move || if saving.get() { "Saving..." } else { "Save Settings" }}
                    </button>
                </div>
            </form>
        </div>
    }
}

/// Helper to get checked state from a checkbox event.
fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // chain_name
    // =========================================================================

    #[test]
    fn test_chain_name_mainnets() {
        assert_eq!(chain_name(1), "Ethereum");
        assert_eq!(chain_name(137), "Polygon");
        assert_eq!(chain_name(42161), "Arbitrum");
        assert_eq!(chain_name(10), "Optimism");
        assert_eq!(chain_name(8453), "Base");
        assert_eq!(chain_name(56), "BSC");
        assert_eq!(chain_name(43114), "Avalanche");
        assert_eq!(chain_name(324), "zkSync");
        assert_eq!(chain_name(59144), "Linea");
        assert_eq!(chain_name(534352), "Scroll");
        assert_eq!(chain_name(100), "Gnosis");
        assert_eq!(chain_name(250), "Fantom");
    }

    #[test]
    fn test_chain_name_testnet() {
        assert_eq!(chain_name(11155111), "Sepolia");
    }

    #[test]
    fn test_chain_name_unknown() {
        assert_eq!(chain_name(0), "Unknown");
        assert_eq!(chain_name(999999), "Unknown");
    }

    // =========================================================================
    // format_date
    // =========================================================================

    #[test]
    fn test_format_date_iso() {
        assert_eq!(format_date("2024-01-15T10:30:00Z"), "Jan 15, 2024");
        assert_eq!(format_date("2024-06-01T00:00:00Z"), "Jun 01, 2024");
        assert_eq!(format_date("2024-12-25T23:59:59Z"), "Dec 25, 2024");
    }

    #[test]
    fn test_format_date_all_months() {
        assert_eq!(format_date("2024-01-01T00:00:00Z"), "Jan 01, 2024");
        assert_eq!(format_date("2024-02-01T00:00:00Z"), "Feb 01, 2024");
        assert_eq!(format_date("2024-03-01T00:00:00Z"), "Mar 01, 2024");
        assert_eq!(format_date("2024-04-01T00:00:00Z"), "Apr 01, 2024");
        assert_eq!(format_date("2024-05-01T00:00:00Z"), "May 01, 2024");
        assert_eq!(format_date("2024-06-01T00:00:00Z"), "Jun 01, 2024");
        assert_eq!(format_date("2024-07-01T00:00:00Z"), "Jul 01, 2024");
        assert_eq!(format_date("2024-08-01T00:00:00Z"), "Aug 01, 2024");
        assert_eq!(format_date("2024-09-01T00:00:00Z"), "Sep 01, 2024");
        assert_eq!(format_date("2024-10-01T00:00:00Z"), "Oct 01, 2024");
        assert_eq!(format_date("2024-11-01T00:00:00Z"), "Nov 01, 2024");
        assert_eq!(format_date("2024-12-01T00:00:00Z"), "Dec 01, 2024");
    }

    #[test]
    fn test_format_date_date_only() {
        assert_eq!(format_date("2024-01-15"), "Jan 15, 2024");
    }

    #[test]
    fn test_format_date_short_string() {
        // Strings shorter than 10 chars returned as-is
        assert_eq!(format_date("2024"), "2024");
        assert_eq!(format_date(""), "");
    }

    #[test]
    fn test_format_date_malformed() {
        // No dashes — split produces 1 part, returned as-is
        assert_eq!(format_date("2024011500"), "2024011500");
    }
}
