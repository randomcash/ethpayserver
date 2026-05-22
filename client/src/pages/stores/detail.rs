//! Store detail page (tab container).

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::EvmApiClient;

use super::general_tab::GeneralTab;
use super::payment_methods_tab::PaymentMethodsTab;
use super::settings_tab::SettingsTab;
use super::webhooks_tab::WebhooksTab;
use super::{IconArchive, IconArrowLeft, format_date};

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
