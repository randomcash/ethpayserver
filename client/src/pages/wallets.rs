//! Wallet management pages - Stripe-inspired design.
//!
//! Wallets contain HD wallet (xpub) configurations that can be used
//! across stores for payment method setup.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::{EvmApiClient, Wallet};

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

/// Truncate address for display.
fn truncate_address(address: &str) -> String {
    if address.len() > 16 {
        format!("{}...{}", &address[..8], &address[address.len()-6..])
    } else {
        address.to_string()
    }
}

/// Wallets list page.
#[component]
pub fn WalletsPage() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let wallets_resource = LocalResource::new(move || {
        let api = api.get();
        async move { api.list_wallets().await }
    });

    view! {
        <div class="wallets-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Wallets"</h1>
                    <p class="page-description">"Manage HD wallets for receiving payments"</p>
                </div>
                <div class="page-actions">
                    <button class="btn btn-primary btn-sm">
                        <IconPlus />
                        "Add wallet"
                    </button>
                </div>
            </div>

            // Wallets Grid
            <Suspense fallback=move || view! { <div class="loading-state">"Loading wallets..."</div> }>
                {move || {
                    wallets_resource.get().map(|result| match &*result {
                        Ok(wallets) if wallets.is_empty() => {
                            view! { <WalletsEmpty /> }.into_any()
                        }
                        Ok(wallets) => {
                            let wallets = wallets.clone();
                            view! {
                                <div class="wallets-grid">
                                    {wallets.into_iter().map(|wallet| {
                                        view! { <WalletCard wallet=wallet /> }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            view! { <div class="error-state">"Error loading wallets: "{msg}</div> }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}

/// Empty state for wallets.
#[component]
fn WalletsEmpty() -> impl IntoView {
    view! {
        <div class="wallets-empty">
            <IconWalletLarge />
            <h3>"No wallets configured"</h3>
            <p>"Add an HD wallet to start receiving cryptocurrency payments"</p>
            <button class="btn btn-primary btn-sm">"Add your first wallet"</button>
        </div>
    }
}

/// Wallet card component.
#[component]
fn WalletCard(wallet: Wallet) -> impl IntoView {
    let wallet_link = wallet.id.clone();
    let wallet_name = wallet.name.clone().unwrap_or_else(|| "Unnamed Wallet".to_string());
    let xpub_display = truncate_address(&wallet.xpub_masked);
    let created_display = format_date(&wallet.created_at);

    view! {
        <A href=format!("/evm/wallets/{}", wallet_link) attr:class="wallet-card">
            <div class="wallet-card-header">
                <div class="wallet-card-icon">
                    <IconWallet />
                </div>
                <div class="wallet-card-title">
                    <h3 class="wallet-card-name">{wallet_name}</h3>
                </div>
            </div>

            <div class="wallet-card-body">
                <div class="wallet-card-address">
                    <code>{xpub_display}</code>
                </div>

                <div class="wallet-card-meta">
                    <div class="wallet-card-row">
                        <span class="wallet-card-label">"Derivation Index"</span>
                        <code class="wallet-card-path">{wallet.derivation_index}</code>
                    </div>
                </div>
            </div>

            <div class="wallet-card-footer">
                <span class="wallet-card-date">"Created "{created_display}</span>
                <IconChevronRight />
            </div>
        </A>
    }
}

/// Wallet detail page.
#[component]
pub fn WalletDetailPage() -> impl IntoView {
    let params = use_params_map();
    let wallet_id = move || params.get().get("id").unwrap_or_default();

    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let wallet_resource = LocalResource::new(move || {
        let api = api.get();
        let id = wallet_id();
        async move { api.get_wallet(&id).await }
    });

    // Active tab state
    let (active_tab, set_active_tab) = signal("general".to_string());

    let tabs = vec![
        ("general", "General"),
        ("addresses", "Addresses"),
    ];

    view! {
        <div class="wallet-detail-page">
            <Suspense fallback=move || view! { <div class="loading-state">"Loading wallet..."</div> }>
                {move || {
                    let active_tab = active_tab;
                    let set_active_tab = set_active_tab;
                    let tabs = tabs.clone();
                    wallet_resource.get().map(move |result| {
                        match &*result {
                            Ok(wallet) => {
                                let wallet = wallet.clone();
                                let wallet_name = wallet.name.clone().unwrap_or_else(|| "Unnamed Wallet".to_string());
                                let created_display = format_date(&wallet.created_at);

                                view! {
                                    <div>
                                        // Header
                                        <div class="wallet-detail-header">
                                            <div class="wallet-detail-header-left">
                                                <A href="/evm/wallets" attr:class="back-link">
                                                    <IconArrowLeft />
                                                    "Wallets"
                                                </A>
                                                <div class="wallet-detail-title-row">
                                                    <h1 class="wallet-detail-title">{wallet_name}</h1>
                                                </div>
                                                <p class="wallet-detail-subtitle">
                                                    <code class="wallet-detail-id">{wallet.id.clone()}</code>
                                                    " · Created "{created_display}
                                                </p>
                                            </div>
                                            <div class="wallet-detail-actions">
                                                <button class="btn btn-secondary btn-sm">
                                                    <IconDownload />
                                                    "Export"
                                                </button>
                                            </div>
                                        </div>

                                        // Tabs
                                        <div class="wallet-tabs">
                                            {tabs.into_iter().map(|(key, label)| {
                                                let key_owned = key.to_string();
                                                let key_for_click = key.to_string();
                                                view! {
                                                    <button
                                                        class=move || if active_tab.get() == key_owned { "wallet-tab active" } else { "wallet-tab" }
                                                        on:click=move |_| set_active_tab.set(key_for_click.clone())
                                                    >
                                                        {label}
                                                    </button>
                                                }
                                            }).collect_view()}
                                        </div>

                                        // Tab Content
                                        <div class="wallet-tab-content">
                                            {move || match active_tab.get().as_str() {
                                                "general" => view! { <GeneralTab wallet=wallet.clone() /> }.into_any(),
                                                "addresses" => view! { <AddressesTab wallet=wallet.clone() /> }.into_any(),
                                                _ => view! { <GeneralTab wallet=wallet.clone() /> }.into_any(),
                                            }}
                                        </div>
                                    </div>
                                }.into_any()
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                view! { <div class="error-state">"Error loading wallet: "{msg}</div> }.into_any()
                            }
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}

/// General settings tab.
#[component]
fn GeneralTab(wallet: Wallet) -> impl IntoView {
    let (name, set_name) = signal(wallet.name.clone().unwrap_or_default());

    view! {
        <div class="wallet-tab-general">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Wallet Information"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Wallet Name"</label>
                        <input
                            type="text"
                            class="form-input"
                            prop:value=move || name.get()
                            on:input=move |ev| set_name.set(event_target_value(&ev))
                        />
                        <p class="form-help">"A friendly name to identify this wallet"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Extended Public Key"</label>
                        <div class="form-static">
                            <code class="wallet-address-full">{wallet.xpub_masked.clone()}</code>
                            <button class="btn btn-ghost btn-sm btn-icon" title="Copy">
                                <IconCopy />
                            </button>
                        </div>
                        <p class="form-help">"Masked xpub used for HD address derivation"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Derivation Index"</label>
                        <div class="form-static">
                            <code>{wallet.derivation_index}</code>
                        </div>
                        <p class="form-help">"Next BIP-44 index for address generation"</p>
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
                            <span class="danger-action-title">"Delete this wallet"</span>
                            <span class="danger-action-desc">"Remove this wallet configuration. This will not affect any funds."</span>
                        </div>
                        <button class="btn btn-danger btn-sm">"Delete wallet"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Addresses tab - shows derived addresses.
#[component]
fn AddressesTab(wallet: Wallet) -> impl IntoView {
    // In production, addresses are derived server-side from the wallet's xpub.
    // For now, show the derivation index range that has been used.
    let used_count = wallet.derivation_index as usize;

    view! {
        <div class="wallet-tab-addresses">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Derived Addresses"</h3>
                    <p class="section-desc">"Addresses generated from your wallet's extended public key"</p>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Addresses Derived"</label>
                        <div class="form-static">
                            <code>{used_count}" addresses derived so far"</code>
                        </div>
                        <p class="form-help">"New addresses are derived automatically when invoices are created"</p>
                    </div>
                </div>
            </div>

            <div class="addresses-info">
                <IconInfo />
                <p>"Addresses are derived sequentially from your wallet's xpub using the BIP-44 standard. Each invoice uses a unique address for payment tracking."</p>
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
fn IconWallet() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12V7H5a2 2 0 0 1 0-4h14v4"></path>
            <path d="M3 5v14a2 2 0 0 0 2 2h16v-5"></path>
            <path d="M18 12a2 2 0 0 0 0 4h4v-4Z"></path>
        </svg>
    }
}

#[component]
fn IconWalletLarge() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12V7H5a2 2 0 0 1 0-4h14v4"></path>
            <path d="M3 5v14a2 2 0 0 0 2 2h16v-5"></path>
            <path d="M18 12a2 2 0 0 0 0 4h4v-4Z"></path>
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
fn IconDownload() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
            <polyline points="7 10 12 15 17 10"></polyline>
            <line x1="12" y1="15" x2="12" y2="3"></line>
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
fn IconInfo() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10"></circle>
            <line x1="12" y1="16" x2="12" y2="12"></line>
            <line x1="12" y1="8" x2="12.01" y2="8"></line>
        </svg>
    }
}
