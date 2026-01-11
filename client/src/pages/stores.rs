//! Store management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{Store, StorePaymentMethod, StoreWebhook};

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
    // Mock stores - will come from API
    let stores: Vec<Store> = vec![
        Store {
            id: "store-001".to_string(),
            name: "My Online Shop".to_string(),
            website: Some("https://myshop.example.com".to_string()),
            archived: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Store {
            id: "store-002".to_string(),
            name: "Demo Store".to_string(),
            website: None,
            archived: false,
            created_at: "2024-01-15T00:00:00Z".to_string(),
        },
        Store {
            id: "store-003".to_string(),
            name: "Test Environment".to_string(),
            website: Some("https://test.example.com".to_string()),
            archived: true,
            created_at: "2023-12-01T00:00:00Z".to_string(),
        },
    ];

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
                    <button class="btn btn-primary btn-sm">
                        <IconPlus />
                        "Create store"
                    </button>
                </div>
            </div>

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

            // Stores Grid - reactive filtering
            <div class="stores-grid">
                {move || {
                    let show = show_archived.get();
                    stores.iter()
                        .filter(|s| show || !s.archived)
                        .cloned()
                        .map(|store| view! { <StoreCard store=store /> })
                        .collect_view()
                }}
            </div>
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
    let params = use_params_map();
    let store_id = move || params.get().get("id").unwrap_or_default();

    // Active tab state
    let (active_tab, set_active_tab) = signal("general".to_string());

    // Mock store data - will come from API
    let store = Store {
        id: store_id(),
        name: "My Online Shop".to_string(),
        website: Some("https://myshop.example.com".to_string()),
        archived: false,
        created_at: "2024-01-01T00:00:00Z".to_string(),
    };

    // Mock payment methods
    let payment_methods: Vec<StorePaymentMethod> = vec![
        StorePaymentMethod {
            id: "pm-001".to_string(),
            store_id: store.id.clone(),
            chain_id: 1,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            decimals: 18,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWZiD6...".to_string(),
            derivation_index: 42,
            enabled: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        },
        StorePaymentMethod {
            id: "pm-002".to_string(),
            store_id: store.id.clone(),
            chain_id: 1,
            token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            asset_symbol: "USDC".to_string(),
            decimals: 6,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWZiD6...".to_string(),
            derivation_index: 15,
            enabled: true,
            created_at: "2024-01-02T00:00:00Z".to_string(),
        },
        StorePaymentMethod {
            id: "pm-003".to_string(),
            store_id: store.id.clone(),
            chain_id: 137,
            token_address: None,
            asset_symbol: "POL".to_string(),
            decimals: 18,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWZiD6...".to_string(),
            derivation_index: 8,
            enabled: true,
            created_at: "2024-01-05T00:00:00Z".to_string(),
        },
        StorePaymentMethod {
            id: "pm-004".to_string(),
            store_id: store.id.clone(),
            chain_id: 42161,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            decimals: 18,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWZiD6...".to_string(),
            derivation_index: 3,
            enabled: false,
            created_at: "2024-01-10T00:00:00Z".to_string(),
        },
    ];

    // Mock webhook
    let webhook: Option<StoreWebhook> = Some(StoreWebhook {
        id: "wh-001".to_string(),
        store_id: store.id.clone(),
        webhook_url: "https://myshop.example.com/api/webhooks/ethpay".to_string(),
        webhook_secret: "whsec_****************************".to_string(),
        enabled: true,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-15T00:00:00Z".to_string(),
    });

    let store_name = store.name.clone();
    let created_display = format_date(&store.created_at);

    let tabs = vec![
        ("general", "General"),
        ("payment_methods", "Payment Methods"),
        ("webhooks", "Webhooks"),
    ];

    view! {
        <div class="store-detail-page">
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
                {tabs.into_iter().map(|(key, label)| {
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
                    "general" => view! { <GeneralTab store=store.clone() /> }.into_any(),
                    "payment_methods" => view! { <PaymentMethodsTab methods=payment_methods.clone() /> }.into_any(),
                    "webhooks" => view! { <WebhooksTab webhook=webhook.clone() /> }.into_any(),
                    _ => view! { <GeneralTab store=store.clone() /> }.into_any(),
                }}
            </div>
        </div>
    }
}

/// General settings tab.
#[component]
fn GeneralTab(store: Store) -> impl IntoView {
    let (name, set_name) = signal(store.name.clone());
    let (website, set_website) = signal(store.website.clone().unwrap_or_default());

    view! {
        <div class="store-tab-general">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Store Information"</h3>
                </div>
                <div class="detail-card-body">
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
                        <button class="btn btn-primary btn-sm">"Save changes"</button>
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
                            <span class="danger-action-title">"Archive this store"</span>
                            <span class="danger-action-desc">"Archived stores cannot receive new payments"</span>
                        </div>
                        <button class="btn btn-danger btn-sm">"Archive store"</button>
                    </div>
                    <div class="danger-action">
                        <div class="danger-action-info">
                            <span class="danger-action-title">"Delete this store"</span>
                            <span class="danger-action-desc">"Permanently delete this store and all its data"</span>
                        </div>
                        <button class="btn btn-danger btn-sm">"Delete store"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Payment methods tab.
#[component]
fn PaymentMethodsTab(methods: Vec<StorePaymentMethod>) -> impl IntoView {
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
fn WebhooksTab(webhook: Option<StoreWebhook>) -> impl IntoView {
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
