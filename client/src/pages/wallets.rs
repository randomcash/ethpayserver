//! Wallet management pages - Stripe-inspired design.
//!
//! Wallets contain HD wallet (xpub) configurations that can be used
//! across stores for payment method setup.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::Wallet;

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
    // Mock wallets - will come from API
    let wallets: Vec<Wallet> = vec![
        Wallet {
            id: "wallet-001".to_string(),
            name: "Main Wallet".to_string(),
            address: "0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D".to_string(),
            derivation_path: "m/44'/60'/0'/0".to_string(),
            enabled_chains: vec![1, 137, 42161, 10],
            created_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Wallet {
            id: "wallet-002".to_string(),
            name: "Cold Storage".to_string(),
            address: "0x1234567890AbCdEf1234567890AbCdEf12345678".to_string(),
            derivation_path: "m/44'/60'/1'/0".to_string(),
            enabled_chains: vec![1],
            created_at: "2024-01-10T00:00:00Z".to_string(),
        },
        Wallet {
            id: "wallet-003".to_string(),
            name: "Test Wallet".to_string(),
            address: "0xDeadBeef00000000000000000000000000000000".to_string(),
            derivation_path: "m/44'/60'/2'/0".to_string(),
            enabled_chains: vec![11155111],
            created_at: "2024-01-15T00:00:00Z".to_string(),
        },
    ];

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
            {if wallets.is_empty() {
                view! { <WalletsEmpty /> }.into_any()
            } else {
                view! {
                    <div class="wallets-grid">
                        {wallets.into_iter().map(|wallet| {
                            view! { <WalletCard wallet=wallet /> }
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
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
    let wallet_name = wallet.name.clone();
    let address_display = truncate_address(&wallet.address);
    let created_display = format_date(&wallet.created_at);
    let chain_count = wallet.enabled_chains.len();

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
                    <code>{address_display}</code>
                </div>

                <div class="wallet-card-meta">
                    <div class="wallet-card-row">
                        <span class="wallet-card-label">"Derivation"</span>
                        <code class="wallet-card-path">{wallet.derivation_path}</code>
                    </div>
                    <div class="wallet-card-row">
                        <span class="wallet-card-label">"Networks"</span>
                        <span class="wallet-card-chains">{chain_count}" chain"{if chain_count != 1 { "s" } else { "" }}</span>
                    </div>
                </div>

                <div class="wallet-card-networks">
                    {wallet.enabled_chains.iter().take(4).map(|&chain_id| {
                        let name = chain_name(chain_id);
                        view! {
                            <span class="wallet-chain-badge">{name}</span>
                        }
                    }).collect_view()}
                    {(wallet.enabled_chains.len() > 4).then(|| {
                        let remaining = wallet.enabled_chains.len() - 4;
                        view! {
                            <span class="wallet-chain-badge wallet-chain-more">"+"{remaining}</span>
                        }
                    })}
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

    // Active tab state
    let (active_tab, set_active_tab) = signal("general".to_string());

    // Mock wallet data - will come from API
    let wallet = Wallet {
        id: wallet_id(),
        name: "Main Wallet".to_string(),
        address: "0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D".to_string(),
        derivation_path: "m/44'/60'/0'/0".to_string(),
        enabled_chains: vec![1, 137, 42161, 10],
        created_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let wallet_name = wallet.name.clone();
    let created_display = format_date(&wallet.created_at);

    let tabs = vec![
        ("general", "General"),
        ("networks", "Networks"),
        ("addresses", "Addresses"),
    ];

    view! {
        <div class="wallet-detail-page">
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
                    "networks" => view! { <NetworksTab wallet=wallet.clone() /> }.into_any(),
                    "addresses" => view! { <AddressesTab wallet=wallet.clone() /> }.into_any(),
                    _ => view! { <GeneralTab wallet=wallet.clone() /> }.into_any(),
                }}
            </div>
        </div>
    }
}

/// General settings tab.
#[component]
fn GeneralTab(wallet: Wallet) -> impl IntoView {
    let (name, set_name) = signal(wallet.name.clone());

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
                        <label class="form-label">"Address"</label>
                        <div class="form-static">
                            <code class="wallet-address-full">{wallet.address.clone()}</code>
                            <button class="btn btn-ghost btn-sm btn-icon" title="Copy">
                                <IconCopy />
                            </button>
                        </div>
                        <p class="form-help">"The primary receiving address derived from your xpub"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Derivation Path"</label>
                        <div class="form-static">
                            <code>{wallet.derivation_path}</code>
                        </div>
                        <p class="form-help">"BIP-44 derivation path used for address generation"</p>
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

/// Networks tab - shows enabled chains.
#[component]
fn NetworksTab(wallet: Wallet) -> impl IntoView {
    let all_networks = vec![
        (1, "Ethereum", "ETH"),
        (137, "Polygon", "POL"),
        (42161, "Arbitrum", "ETH"),
        (10, "Optimism", "ETH"),
        (8453, "Base", "ETH"),
        (56, "BSC", "BNB"),
        (43114, "Avalanche", "AVAX"),
        (324, "zkSync Era", "ETH"),
        (59144, "Linea", "ETH"),
        (534352, "Scroll", "ETH"),
        (100, "Gnosis", "xDAI"),
        (250, "Fantom", "FTM"),
    ];

    view! {
        <div class="wallet-tab-networks">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Enabled Networks"</h3>
                    <p class="section-desc">"Select which networks this wallet can receive payments on"</p>
                </div>
            </div>

            <div class="networks-grid">
                {all_networks.into_iter().map(|(chain_id, name, symbol)| {
                    let is_enabled = wallet.enabled_chains.contains(&chain_id);
                    let status_class = if is_enabled { "network-card enabled" } else { "network-card" };

                    view! {
                        <div class=status_class>
                            <div class="network-card-header">
                                <span class="network-card-name">{name}</span>
                                <span class="network-card-symbol">{symbol}</span>
                            </div>
                            <div class="network-card-footer">
                                <span class="network-card-chain-id">"Chain ID: "{chain_id}</span>
                                <label class="toggle">
                                    <input type="checkbox" checked=is_enabled />
                                    <span class="toggle-slider"></span>
                                </label>
                            </div>
                        </div>
                    }
                }).collect_view()}
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Testnets"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="testnet-row">
                        <div>
                            <span class="testnet-name">"Sepolia"</span>
                            <span class="testnet-chain-id">"Chain ID: 11155111"</span>
                        </div>
                        <label class="toggle">
                            <input type="checkbox" checked=wallet.enabled_chains.contains(&11155111) />
                            <span class="toggle-slider"></span>
                        </label>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Addresses tab - shows derived addresses.
#[component]
fn AddressesTab(wallet: Wallet) -> impl IntoView {
    // Mock derived addresses - in production, derive from wallet.derivation_path
    let _derivation_base = &wallet.derivation_path;
    let addresses = vec![
        (0, "0x742d35Cc6634C0532925a3b844Bc9e7595f0Ab3D", true),
        (1, "0x8Ba1f109551bD432803012645Ac136ddd64DBA72", true),
        (2, "0xdD2FD4581271e230360230F9337D5c0430Bf44C0", true),
        (3, "0x2546BcD3c84621e976D8185a91A922aE77ECEc30", false),
        (4, "0xbDA5747bFD65F08deb54cb465eB87D40e51B197E", false),
    ];

    view! {
        <div class="wallet-tab-addresses">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"Derived Addresses"</h3>
                    <p class="section-desc">"Addresses generated from your wallet's extended public key"</p>
                </div>
                <button class="btn btn-secondary btn-sm">
                    <IconPlus />
                    "Derive more"
                </button>
            </div>

            <div class="addresses-table-container">
                <table class="addresses-table">
                    <thead>
                        <tr>
                            <th>"Index"</th>
                            <th>"Address"</th>
                            <th>"Status"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        {addresses.into_iter().map(|(index, address, used)| {
                            let status_class = if used { "badge badge-neutral" } else { "badge badge-success" };
                            let status_label = if used { "Used" } else { "Available" };

                            view! {
                                <tr class="address-row">
                                    <td>
                                        <code class="address-index">{index}</code>
                                    </td>
                                    <td>
                                        <code class="address-full">{address}</code>
                                    </td>
                                    <td>
                                        <span class=status_class>{status_label}</span>
                                    </td>
                                    <td>
                                        <button class="btn btn-ghost btn-sm btn-icon" title="Copy">
                                            <IconCopy />
                                        </button>
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
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
