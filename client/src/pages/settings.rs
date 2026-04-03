//! Settings page - Stripe-inspired design with multiple tabs.
//!
//! Contains user settings, preferences, API keys, and admin-only server settings.

use leptos::prelude::*;

use crate::api::{EvmApiClient, UserRole};

/// Settings page with tabbed interface.
#[component]
pub fn SettingsPage() -> impl IntoView {
    // Active tab state
    let (active_tab, set_active_tab) = signal("account".to_string());

    // Load user role to conditionally show admin tab
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let (is_admin, set_is_admin) = signal(false);

    leptos::task::spawn_local({
        let api = api.get_untracked();
        async move {
            if let Ok(user) = api.get_me().await {
                set_is_admin.set(user.role.is_admin());
            }
        }
    });

    // Base tabs available to all users
    let base_tabs: Vec<(&str, &str)> = vec![
        ("account", "Account"),
        ("preferences", "Preferences"),
        ("api_keys", "API Keys"),
        ("notifications", "Notifications"),
    ];

    view! {
        <div class="settings-page">
            // Page Header
            <div class="page-header-row">
                <div>
                    <h1 class="page-title">"Settings"</h1>
                    <p class="page-description">"Manage your account and preferences"</p>
                </div>
            </div>

            // Tabs
            <div class="settings-tabs">
                {base_tabs.into_iter().map(|(key, label)| {
                    let key_owned = key.to_string();
                    let key_for_click = key.to_string();
                    view! {
                        <button
                            class=move || {
                                if active_tab.get() == key_owned {
                                    "settings-tab active".to_string()
                                } else {
                                    "settings-tab".to_string()
                                }
                            }
                            on:click=move |_| set_active_tab.set(key_for_click.clone())
                        >
                            <span class="settings-tab-label">{label}</span>
                        </button>
                    }
                }).collect_view()}
                // Admin tab - only shown for server admins
                <Show when=move || is_admin.get()>
                    <button
                        class=move || {
                            if active_tab.get() == "admin" {
                                "settings-tab active settings-tab-admin".to_string()
                            } else {
                                "settings-tab settings-tab-admin".to_string()
                            }
                        }
                        on:click=move |_| set_active_tab.set("admin".to_string())
                    >
                        <span class="settings-tab-label">
                            "Server Admin"
                            <IconShield />
                        </span>
                    </button>
                </Show>
            </div>

            // Tab Content
            <div class="settings-tab-content">
                {move || match active_tab.get().as_str() {
                    "account" => view! { <AccountTab /> }.into_any(),
                    "preferences" => view! { <PreferencesTab /> }.into_any(),
                    "api_keys" => view! { <ApiKeysTab /> }.into_any(),
                    "notifications" => view! { <NotificationsTab /> }.into_any(),
                    "admin" if is_admin.get() => view! { <AdminTab /> }.into_any(),
                    _ => view! { <AccountTab /> }.into_any(),
                }}
            </div>
        </div>
    }
}

/// Format an ISO 8601 date string for display.
fn format_date(iso: &str) -> String {
    if let Some(date_part) = iso.split('T').next() {
        date_part.to_string()
    } else {
        iso.to_string()
    }
}

/// Account settings tab — loads profile from `/auth/me`.
#[component]
fn AccountTab() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    let user_resource = LocalResource::new(move || {
        let api = api.get();
        async move { api.get_me().await }
    });

    view! {
        <div class="settings-tab-account">
            <Suspense fallback=move || view! {
                <div style="text-align: center; padding: 3rem; color: var(--text-muted);">
                    "Loading profile..."
                </div>
            }>
                {move || user_resource.get().map(|result| match &*result {
                    Ok(user) => {
                        let email_display = user.email.clone().unwrap_or_else(|| "Not set".to_string());
                        let wallet_display = user.primary_wallet_address.clone().unwrap_or_else(|| "Not set".to_string());
                        let role_label = user.role.label().to_string();
                        let role_class = if user.role.is_admin() { "badge badge-warning" } else { "badge badge-neutral" };
                        let created = format_date(&user.created_at);
                        let last_login = user.last_login_at.as_deref().map(format_date).unwrap_or_else(|| "Never".to_string());
                        let user_id = user.id.clone();
                        let has_email = user.email.is_some();
                        let has_wallet = user.primary_wallet_address.is_some();

                        view! {
                            <div class="detail-card">
                                <div class="detail-card-header">
                                    <h3>"Profile Information"</h3>
                                    <span class=role_class>{role_label}</span>
                                </div>
                                <div class="detail-card-body">
                                    <div class="form-group">
                                        <label class="form-label">"User ID"</label>
                                        <div class="form-static">
                                            <code>{user_id}</code>
                                        </div>
                                    </div>

                                    <div class="form-group">
                                        <label class="form-label">"Email Address"</label>
                                        <div class="form-static">
                                            {if has_email {
                                                view! { <span>{email_display}</span> }.into_any()
                                            } else {
                                                view! { <span class="text-muted">"Not set (wallet-only account)"</span> }.into_any()
                                            }}
                                        </div>
                                    </div>

                                    <div class="form-group">
                                        <label class="form-label">"Wallet Address"</label>
                                        <div class="form-static">
                                            {if has_wallet {
                                                view! { <code>{wallet_display}</code> }.into_any()
                                            } else {
                                                view! { <span class="text-muted">"Not set (email-only account)"</span> }.into_any()
                                            }}
                                        </div>
                                    </div>

                                    <div class="settings-grid">
                                        <div class="form-group">
                                            <label class="form-label">"Account Created"</label>
                                            <div class="form-static">{created}</div>
                                        </div>
                                        <div class="form-group">
                                            <label class="form-label">"Last Login"</label>
                                            <div class="form-static">{last_login}</div>
                                        </div>
                                    </div>
                                </div>
                            </div>

                            <div class="detail-card">
                                <div class="detail-card-header">
                                    <h3>"Security"</h3>
                                </div>
                                <div class="detail-card-body">
                                    <p class="form-help" style="margin-bottom: 16px;">
                                        "This server uses passwordless authentication. Manage your passkeys and connected wallets to control access to your account."
                                    </p>
                                    <div class="form-actions">
                                        <button class="btn btn-secondary btn-sm" disabled=true>
                                            "Manage passkeys"
                                        </button>
                                        <button class="btn btn-secondary btn-sm" disabled=true>
                                            "Manage wallets"
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
                                            <span class="danger-action-title">"Delete account"</span>
                                            <span class="danger-action-desc">"Permanently delete your account and all associated data"</span>
                                        </div>
                                        <button class="btn btn-danger btn-sm" disabled=true>"Delete account"</button>
                                    </div>
                                </div>
                            </div>
                        }.into_any()
                    }
                    Err(e) => view! {
                        <div class="detail-card">
                            <div class="detail-card-body">
                                <p class="text-error">"Failed to load profile: "{e.to_string()}</p>
                            </div>
                        </div>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

/// Preferences tab.
#[component]
fn PreferencesTab() -> impl IntoView {
    let (theme, set_theme) = signal("system".to_string());
    let (currency, set_currency) = signal("USD".to_string());
    let (timezone, set_timezone) = signal("UTC".to_string());
    let (date_format, set_date_format) = signal("mdy".to_string());

    view! {
        <div class="settings-tab-preferences">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Appearance"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Theme"</label>
                        <select
                            class="form-select"
                            prop:value=move || theme.get()
                            on:change=move |ev| set_theme.set(event_target_value(&ev))
                        >
                            <option value="system">"System default"</option>
                            <option value="light">"Light"</option>
                            <option value="dark">"Dark"</option>
                        </select>
                    </div>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Regional Settings"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Default Currency"</label>
                        <select
                            class="form-select"
                            prop:value=move || currency.get()
                            on:change=move |ev| set_currency.set(event_target_value(&ev))
                        >
                            <option value="USD">"USD - US Dollar"</option>
                            <option value="EUR">"EUR - Euro"</option>
                            <option value="GBP">"GBP - British Pound"</option>
                            <option value="JPY">"JPY - Japanese Yen"</option>
                            <option value="BTC">"BTC - Bitcoin"</option>
                            <option value="ETH">"ETH - Ethereum"</option>
                        </select>
                        <p class="form-help">"Used for displaying amounts in your preferred currency"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Timezone"</label>
                        <select
                            class="form-select"
                            prop:value=move || timezone.get()
                            on:change=move |ev| set_timezone.set(event_target_value(&ev))
                        >
                            <option value="UTC">"UTC"</option>
                            <option value="America/New_York">"Eastern Time (US)"</option>
                            <option value="America/Los_Angeles">"Pacific Time (US)"</option>
                            <option value="Europe/London">"London"</option>
                            <option value="Europe/Paris">"Paris"</option>
                            <option value="Asia/Tokyo">"Tokyo"</option>
                            <option value="Asia/Shanghai">"Shanghai"</option>
                        </select>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Date Format"</label>
                        <select
                            class="form-select"
                            prop:value=move || date_format.get()
                            on:change=move |ev| set_date_format.set(event_target_value(&ev))
                        >
                            <option value="mdy">"MM/DD/YYYY"</option>
                            <option value="dmy">"DD/MM/YYYY"</option>
                            <option value="ymd">"YYYY-MM-DD"</option>
                        </select>
                    </div>

                    <div class="form-actions">
                        <button class="btn btn-primary btn-sm">"Save preferences"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// API Keys tab.
#[component]
fn ApiKeysTab() -> impl IntoView {
    // Mock API keys
    let api_keys = vec![
        ("ak_live_****1234", "Production Key", "Jan 15, 2024", true),
        ("ak_test_****5678", "Test Key", "Jan 10, 2024", true),
        ("ak_live_****9012", "Old Key", "Dec 1, 2023", false),
    ];

    view! {
        <div class="settings-tab-api-keys">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"API Keys"</h3>
                    <p class="section-desc">"Manage API keys for programmatic access"</p>
                </div>
                <button class="btn btn-primary btn-sm">
                    <IconPlus />
                    "Create API key"
                </button>
            </div>

            <div class="api-keys-list">
                {api_keys.into_iter().map(|(key, name, created, active)| {
                    let status_class = if active { "badge badge-success" } else { "badge badge-neutral" };
                    let status_label = if active { "Active" } else { "Revoked" };

                    view! {
                        <div class="api-key-item">
                            <div class="api-key-info">
                                <div class="api-key-header">
                                    <span class="api-key-name">{name}</span>
                                    <span class=status_class>{status_label}</span>
                                </div>
                                <code class="api-key-value">{key}</code>
                                <span class="api-key-created">"Created "{created}</span>
                            </div>
                            <div class="api-key-actions">
                                {active.then(|| view! {
                                    <button class="btn btn-ghost btn-sm">"Revoke"</button>
                                })}
                                <button class="btn btn-ghost btn-sm btn-icon" title="Copy">
                                    <IconCopy />
                                </button>
                            </div>
                        </div>
                    }
                }).collect_view()}
            </div>

            <div class="settings-info">
                <IconInfo />
                <div>
                    <p><strong>"Keep your API keys secure"</strong></p>
                    <p>"Never share your API keys publicly or commit them to version control. Use environment variables in production."</p>
                </div>
            </div>
        </div>
    }
}

/// Notifications tab.
#[component]
fn NotificationsTab() -> impl IntoView {
    let (email_payments, set_email_payments) = signal(true);
    let (email_invoices, set_email_invoices) = signal(true);
    let (email_security, set_email_security) = signal(true);
    let (email_marketing, set_email_marketing) = signal(false);

    view! {
        <div class="settings-tab-notifications">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Email Notifications"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="notification-options">
                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Payment notifications"</span>
                                <span class="notification-option-desc">"Get notified when payments are received or confirmed"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_payments.get()
                                    on:change=move |ev| set_email_payments.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>

                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Invoice updates"</span>
                                <span class="notification-option-desc">"Notifications for invoice status changes"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_invoices.get()
                                    on:change=move |ev| set_email_invoices.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>

                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Security alerts"</span>
                                <span class="notification-option-desc">"Important security notifications and login alerts"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_security.get()
                                    on:change=move |ev| set_email_security.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>

                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Product updates"</span>
                                <span class="notification-option-desc">"News about new features and improvements"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_marketing.get()
                                    on:change=move |ev| set_email_marketing.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>
                    </div>

                    <div class="form-actions">
                        <button class="btn btn-primary btn-sm">"Save notification settings"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}

/// Admin tab - server settings (admin only).
#[component]
fn AdminTab() -> impl IntoView {
    let (server_url, set_server_url) = signal("https://api.ethpayserver.local".to_string());
    let (redis_url, set_redis_url) = signal("redis://localhost:6379".to_string());
    let (db_url, set_db_url) = signal("postgresql://localhost/ethpayserver".to_string());
    let (default_confirmations, set_default_confirmations) = signal("12".to_string());
    let (invoice_expiry, set_invoice_expiry) = signal("60".to_string());
    let (rate_limit, set_rate_limit) = signal("100".to_string());

    // Mock enabled networks
    let all_networks = vec![
        (1, "Ethereum", true),
        (137, "Polygon", true),
        (42161, "Arbitrum", true),
        (10, "Optimism", true),
        (8453, "Base", true),
        (56, "BSC", false),
        (43114, "Avalanche", false),
        (324, "zkSync Era", false),
        (59144, "Linea", false),
        (534352, "Scroll", false),
        (100, "Gnosis", false),
        (250, "Fantom", false),
    ];

    view! {
        <div class="settings-tab-admin">
            <div class="admin-warning">
                <IconShield />
                <div>
                    <strong>"Server Administration"</strong>
                    <p>"These settings affect the entire server. Changes may require a restart to take effect."</p>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Server Configuration"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Server URL"</label>
                        <input
                            type="url"
                            class="form-input"
                            prop:value=move || server_url.get()
                            on:input=move |ev| set_server_url.set(event_target_value(&ev))
                        />
                        <p class="form-help">"Public URL of this server"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Redis URL"</label>
                        <input
                            type="text"
                            class="form-input form-input-mono"
                            prop:value=move || redis_url.get()
                            on:input=move |ev| set_redis_url.set(event_target_value(&ev))
                        />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Database URL"</label>
                        <input
                            type="text"
                            class="form-input form-input-mono"
                            prop:value=move || db_url.get()
                            on:input=move |ev| set_db_url.set(event_target_value(&ev))
                        />
                    </div>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Payment Defaults"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="settings-grid">
                        <div class="form-group">
                            <label class="form-label">"Required Confirmations"</label>
                            <input
                                type="number"
                                class="form-input"
                                min="1"
                                max="100"
                                prop:value=move || default_confirmations.get()
                                on:input=move |ev| set_default_confirmations.set(event_target_value(&ev))
                            />
                            <p class="form-help">"Block confirmations before payment is final"</p>
                        </div>

                        <div class="form-group">
                            <label class="form-label">"Invoice Expiry (minutes)"</label>
                            <input
                                type="number"
                                class="form-input"
                                min="5"
                                max="1440"
                                prop:value=move || invoice_expiry.get()
                                on:input=move |ev| set_invoice_expiry.set(event_target_value(&ev))
                            />
                            <p class="form-help">"Default expiration time for new invoices"</p>
                        </div>

                        <div class="form-group">
                            <label class="form-label">"Rate Limit (req/min)"</label>
                            <input
                                type="number"
                                class="form-input"
                                min="10"
                                max="1000"
                                prop:value=move || rate_limit.get()
                                on:input=move |ev| set_rate_limit.set(event_target_value(&ev))
                            />
                            <p class="form-help">"API rate limit per IP address"</p>
                        </div>
                    </div>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Enabled Networks"</h3>
                </div>
                <div class="detail-card-body">
                    <p class="form-help" style="margin-bottom: 16px;">
                        "Networks available for payment processing. Disabled networks cannot be used by any store."
                    </p>
                    <div class="admin-networks-grid">
                        {all_networks.into_iter().map(|(chain_id, name, enabled)| {
                            view! {
                                <div class=if enabled { "admin-network-item enabled" } else { "admin-network-item" }>
                                    <div class="admin-network-info">
                                        <span class="admin-network-name">{name}</span>
                                        <span class="admin-network-chain-id">"Chain ID: "{chain_id}</span>
                                    </div>
                                    <label class="toggle">
                                        <input type="checkbox" checked=enabled />
                                        <span class="toggle-slider"></span>
                                    </label>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Maintenance"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="admin-actions">
                        <div class="admin-action">
                            <div class="admin-action-info">
                                <span class="admin-action-title">"Clear cache"</span>
                                <span class="admin-action-desc">"Clear all cached data including exchange rates"</span>
                            </div>
                            <button class="btn btn-secondary btn-sm">"Clear cache"</button>
                        </div>

                        <div class="admin-action">
                            <div class="admin-action-info">
                                <span class="admin-action-title">"Restart monitor"</span>
                                <span class="admin-action-desc">"Restart the EVM chain monitor service"</span>
                            </div>
                            <button class="btn btn-secondary btn-sm">"Restart"</button>
                        </div>

                        <div class="admin-action">
                            <div class="admin-action-info">
                                <span class="admin-action-title">"Export data"</span>
                                <span class="admin-action-desc">"Export all server data as JSON backup"</span>
                            </div>
                            <button class="btn btn-secondary btn-sm">"Export"</button>
                        </div>
                    </div>
                </div>
            </div>

            <div class="form-actions">
                <button class="btn btn-primary">"Save server settings"</button>
            </div>
        </div>
    }
}

// ============================================
// Icons
// ============================================

#[component]
fn IconShield() -> impl IntoView {
    view! {
        <svg class="settings-tab-icon" xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
        </svg>
    }
}

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
