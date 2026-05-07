//! Settings page - Stripe-inspired design with multiple tabs.
//!
//! Contains user settings, preferences, API keys, and admin-only server settings.

use crate::api::{
    AdminUserInfo, ApiKeyInfo, CreateApiKeyRequest, CreateApiKeyResponsePayload, EvmApiClient,
    RotateApiKeyResponse, UpdateServerSettingsRequest, UpdateUserRoleRequest,
};
use leptos::prelude::*;

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
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    // Track version to trigger refetches after create/revoke
    let (version, set_version) = signal(0u32);

    // Load API keys from backend
    let keys_resource = LocalResource::new(move || {
        let client = api.get();
        let _v = version.get();
        async move { client.list_api_keys().await.ok() }
    });

    // State for create form
    let (show_create, set_show_create) = signal(false);
    let (new_key_name, set_new_key_name) = signal(String::new());
    let (created_key, set_created_key) = signal(Option::<CreateApiKeyResponsePayload>::None);
    let (loading, set_loading) = signal(false);

    // Create handler
    let on_create = move |_| {
        let name = new_key_name.get();
        if name.trim().is_empty() {
            return;
        }
        let client = api.get();
        let request = CreateApiKeyRequest {
            name: name.trim().to_string(),
            expires_at: None,
        };
        set_loading.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(resp) = client.create_api_key(&request).await {
                set_created_key.set(Some(resp));
                set_show_create.set(false);
                set_new_key_name.set(String::new());
                set_version.update(|v| *v += 1);
            }
            set_loading.set(false);
        });
    };

    // Revoke handler factory
    let make_revoke_handler = move |key_id: String| {
        let client = api.get();
        move |_| {
            let client = client.clone();
            let id = key_id.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = client.revoke_api_key(&id).await;
                set_version.update(|v| *v += 1);
            });
        }
    };

    // State for rotated key display
    let (rotated_key, set_rotated_key) = signal(Option::<RotateApiKeyResponse>::None);
    // Error surfaced on a failed rotation — previously the handler swallowed
    // errors silently, leaving the user wondering if the click had any effect.
    let (rotate_error, set_rotate_error) = signal(Option::<String>::None);

    // Rotate handler factory
    let make_rotate_handler = move |key_id: String| {
        let client = api.get();
        move |_| {
            let client = client.clone();
            let id = key_id.clone();
            set_rotate_error.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                match client.rotate_api_key(&id).await {
                    Ok(resp) => {
                        set_rotated_key.set(Some(resp));
                        set_version.update(|v| *v += 1);
                    }
                    Err(err) => {
                        set_rotate_error.set(Some(format!("Failed to rotate key: {err}")));
                    }
                }
            });
        }
    };

    view! {
        <div class="settings-tab-api-keys">
            <div class="section-header">
                <div>
                    <h3 class="section-title">"API Keys"</h3>
                    <p class="section-desc">"Manage API keys for programmatic access"</p>
                </div>
                <button
                    class="btn btn-primary btn-sm"
                    on:click=move |_| set_show_create.set(true)
                >
                    <IconPlus />
                    "Create API key"
                </button>
            </div>

            // Create form
            {move || show_create.get().then(|| view! {
                <div class="detail-card" style="margin-bottom: 16px;">
                    <div class="detail-card-header">
                        <h3>"Create new API key"</h3>
                    </div>
                    <div class="detail-card-body">
                        <div class="form-group">
                            <label class="form-label">"Key name"</label>
                            <input
                                type="text"
                                class="form-input"
                                placeholder="e.g., Production Key"
                                prop:value=move || new_key_name.get()
                                on:input=move |ev| set_new_key_name.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="form-actions">
                            <button
                                class="btn btn-primary btn-sm"
                                prop:disabled=move || loading.get()
                                on:click=on_create
                            >
                                {move || if loading.get() { "Creating..." } else { "Create key" }}
                            </button>
                            <button
                                class="btn btn-ghost btn-sm"
                                on:click=move |_| set_show_create.set(false)
                            >
                                "Cancel"
                            </button>
                        </div>
                    </div>
                </div>
            })}

            // Show newly created key (plaintext shown once)
            {move || created_key.get().map(|key| view! {
                <div class="detail-card" style="margin-bottom: 16px; border-color: var(--color-success);">
                    <div class="detail-card-body">
                        <p><strong>"Your new API key has been created. Copy it now — it will not be shown again."</strong></p>
                        <code class="api-key-value" style="display: block; margin: 8px 0; padding: 8px; background: var(--color-bg-secondary); word-break: break-all;">
                            {key.key.clone()}
                        </code>
                        <button
                            class="btn btn-ghost btn-sm"
                            on:click=move |_| set_created_key.set(None)
                        >
                            "Dismiss"
                        </button>
                    </div>
                </div>
            })}

            // Rotation error — user must know when a rotate click failed.
            {move || rotate_error.get().map(|msg| view! {
                <div class="detail-card" style="margin-bottom: 16px; border-color: var(--color-danger);">
                    <div class="detail-card-body">
                        <p><strong>{msg}</strong></p>
                        <button
                            class="btn btn-ghost btn-sm"
                            on:click=move |_| set_rotate_error.set(None)
                        >
                            "Dismiss"
                        </button>
                    </div>
                </div>
            })}

            // Show rotated key (plaintext shown once)
            {move || rotated_key.get().map(|key| {
                let grace_line = key.old_key_grace_expires_at
                    .as_deref()
                    .map(|exp| format!("The old key remains valid until {exp}."))
                    .unwrap_or_else(|| "The old key remains valid during the grace period.".to_string());
                view! {
                <div class="detail-card" style="margin-bottom: 16px; border-color: var(--color-warning);">
                    <div class="detail-card-body">
                        <p><strong>"Key rotated successfully. Copy your new key now — it will not be shown again."</strong></p>
                        <p>{grace_line}</p>
                        <code class="api-key-value" style="display: block; margin: 8px 0; padding: 8px; background: var(--color-bg-secondary); word-break: break-all;">
                            {key.key.clone()}
                        </code>
                        <button
                            class="btn btn-ghost btn-sm"
                            on:click=move |_| set_rotated_key.set(None)
                        >
                            "Dismiss"
                        </button>
                    </div>
                </div>
            }})}

            // API keys list
            <Suspense fallback=move || view! { <p>"Loading API keys..."</p> }>
                {move || Suspend::new(async move {
                    match keys_resource.await {
                        Some(resp) => {
                            let keys = resp.keys;
                            if keys.is_empty() {
                                view! { <p class="empty-state">"No API keys yet. Create one to get started."</p> }.into_any()
                            } else {
                                view! {
                                    <div class="api-keys-list">
                                        {keys.into_iter().map(|key: ApiKeyInfo| {
                                            let (status_class, status_label) = if !key.is_active {
                                                ("badge badge-neutral".to_string(), "Revoked".to_string())
                                            } else if key.deprecated_at.is_some() {
                                                // Surface the actual expiry rather than a
                                                // static "Deprecated" — users need to know
                                                // when the grace window ends.
                                                let label = match key.deprecation_expires_at.as_deref() {
                                                    Some(exp) => format!("Deprecated — expires {exp}"),
                                                    None => "Deprecated".to_string(),
                                                };
                                                ("badge badge-warning".to_string(), label)
                                            } else {
                                                ("badge badge-success".to_string(), "Active".to_string())
                                            };
                                            let is_active = key.is_active;
                                            let is_deprecated = key.deprecated_at.is_some();
                                            let revoke_handler = make_revoke_handler(key.id.clone());
                                            let rotate_handler = make_rotate_handler(key.id.clone());

                                            view! {
                                                <div class="api-key-item">
                                                    <div class="api-key-info">
                                                        <div class="api-key-header">
                                                            <span class="api-key-name">{key.name}</span>
                                                            <span class=status_class>{status_label}</span>
                                                        </div>
                                                        <code class="api-key-value">{key.key_prefix}</code>
                                                        <span class="api-key-created">"Created "{key.created_at}</span>
                                                    </div>
                                                    <div class="api-key-actions">
                                                        {(is_active && !is_deprecated).then(|| view! {
                                                            <button
                                                                class="btn btn-ghost btn-sm"
                                                                on:click=rotate_handler
                                                            >
                                                                "Rotate"
                                                            </button>
                                                        })}
                                                        {is_active.then(|| view! {
                                                            <button
                                                                class="btn btn-ghost btn-sm"
                                                                on:click=revoke_handler
                                                            >
                                                                "Revoke"
                                                            </button>
                                                        })}
                                                    </div>
                                                </div>
                                            }
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            }
                        }
                        None => view! { <p class="error">"Failed to load API keys"</p> }.into_any(),
                    }
                })}
            </Suspense>

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

/// Admin tab - server settings and user management (admin only).
#[component]
fn AdminTab() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");

    // Settings form state
    let (default_confirmations, set_default_confirmations) = signal("3".to_string());
    let (invoice_expiry, set_invoice_expiry) = signal("60".to_string());
    let (rate_limit, set_rate_limit) = signal("100".to_string());
    let (enabled_chain_ids, set_enabled_chain_ids) = signal(Vec::<i64>::new());
    let (settings_status, set_settings_status) = signal(String::new());

    // User list state
    let (users, set_users) = signal(Vec::<AdminUserInfo>::new());
    let (user_total, set_user_total) = signal(0i64);
    let (user_status, set_user_status) = signal(String::new());

    // All available networks
    let all_networks: Vec<(i64, &'static str)> = vec![
        (1, "Ethereum"),
        (10, "Optimism"),
        (137, "Polygon"),
        (42161, "Arbitrum"),
        (8453, "Base"),
        (56, "BSC"),
        (43114, "Avalanche"),
        (250, "Fantom"),
        (100, "Gnosis"),
        (324, "zkSync Era"),
        (59144, "Linea"),
        (534352, "Scroll"),
    ];

    // Load settings and users on mount
    leptos::task::spawn_local({
        let api = api.get_untracked();
        async move {
            if let Ok(settings) = api.get_server_settings().await {
                set_default_confirmations.set(settings.default_confirmations.to_string());
                set_invoice_expiry.set(settings.invoice_expiry_minutes.to_string());
                set_rate_limit.set(settings.rate_limit_rpm.to_string());
                set_enabled_chain_ids.set(settings.enabled_chain_ids);
            }
            if let Ok(resp) = api.list_users(0, 100).await {
                set_user_total.set(resp.total);
                set_users.set(resp.users);
            }
        }
    });

    // Save settings handler
    let save_settings = move |_| {
        let api = api.get_untracked();
        let confirmations = default_confirmations
            .get_untracked()
            .parse::<i32>()
            .unwrap_or(3);
        let expiry = invoice_expiry.get_untracked().parse::<i32>().unwrap_or(60);
        let rpm = rate_limit.get_untracked().parse::<i32>().unwrap_or(100);
        let chains = enabled_chain_ids.get_untracked();
        leptos::task::spawn_local(async move {
            let request = UpdateServerSettingsRequest {
                default_confirmations: confirmations,
                invoice_expiry_minutes: expiry,
                rate_limit_rpm: rpm,
                enabled_chain_ids: chains,
            };
            match api.update_server_settings(&request).await {
                Ok(()) => set_settings_status.set("Settings saved".to_string()),
                Err(e) => set_settings_status.set(format!("Error: {}", e)),
            }
        });
    };

    // Toggle network handler
    let toggle_network = move |chain_id: i64| {
        set_enabled_chain_ids.update(|ids| {
            if ids.contains(&chain_id) {
                ids.retain(|&id| id != chain_id);
            } else {
                ids.push(chain_id);
            }
        });
    };

    // Role change handler
    let change_role = move |user_id: String, new_role: String| {
        let api = api.get_untracked();
        leptos::task::spawn_local(async move {
            let request = UpdateUserRoleRequest { role: new_role };
            match api.update_user_role(&user_id, &request).await {
                Ok(()) => {
                    set_user_status.set("Role updated".to_string());
                    if let Ok(resp) = api.list_users(0, 100).await {
                        set_users.set(resp.users);
                    }
                }
                Err(e) => set_user_status.set(format!("Error: {}", e)),
            }
        });
    };

    // Lock/unlock handler
    let toggle_lock = move |user_id: String, is_locked: bool| {
        let api = api.get_untracked();
        leptos::task::spawn_local(async move {
            let result = if is_locked {
                api.unlock_user(&user_id).await
            } else {
                api.lock_user(&user_id).await
            };
            match result {
                Ok(()) => {
                    if let Ok(resp) = api.list_users(0, 100).await {
                        set_users.set(resp.users);
                    }
                }
                Err(e) => set_user_status.set(format!("Error: {}", e)),
            }
        });
    };

    view! {
        <div class="settings-tab-admin">
            <div class="admin-warning">
                <IconShield />
                <div>
                    <strong>"Server Administration"</strong>
                    <p>"These settings affect the entire server. Changes may require a restart to take effect."</p>
                </div>
            </div>

            // User Management
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"User Management"</h3>
                    <span class="badge">{move || format!("{} users", user_total.get())}</span>
                </div>
                <div class="detail-card-body">
                    {move || {
                        let status = user_status.get();
                        if status.is_empty() {
                            view! { <span></span> }.into_any()
                        } else {
                            view! { <p class="form-help">{status}</p> }.into_any()
                        }
                    }}
                    <div class="admin-users-table">
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"User"</th>
                                    <th>"Role"</th>
                                    <th>"Created"</th>
                                    <th>"Actions"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {move || users.get().into_iter().map(|user| {
                                    let user_id_role = user.id.clone();
                                    let user_id_lock = user.id.clone();
                                    let is_locked = user.locked_until.is_some();
                                    let display_name = user.email.clone()
                                        .or(user.primary_wallet_address.clone().map(|w| {
                                            let prefix = w.get(..6).unwrap_or(w.as_str());
                                            let suffix = w.get(w.len().saturating_sub(4)..).unwrap_or("");
                                            format!("{prefix}...{suffix}")
                                        }))
                                        .unwrap_or_else(|| user.id.get(..8).unwrap_or(user.id.as_str()).to_string());
                                    let current_role = user.role.clone();
                                    let created_date = user.created_at.get(..10).unwrap_or(&user.created_at).to_string();
                                    view! {
                                        <tr class=if is_locked { "user-row locked" } else { "user-row" }>
                                            <td class="user-cell">
                                                <span class="user-name">{display_name}</span>
                                            </td>
                                            <td>
                                                <select
                                                    class="form-input form-input-sm"
                                                    on:change=move |ev| {
                                                        let new_role = event_target_value(&ev);
                                                        change_role(user_id_role.clone(), new_role);
                                                    }
                                                >
                                                    <option value="server_admin" selected=current_role == "server_admin">"Server Admin"</option>
                                                    <option value="user" selected=current_role == "user">"User"</option>
                                                </select>
                                            </td>
                                            <td class="date-cell">{created_date}</td>
                                            <td>
                                                <button
                                                    class=if is_locked { "btn btn-xs btn-success" } else { "btn btn-xs btn-warning" }
                                                    on:click=move |_| toggle_lock(user_id_lock.clone(), is_locked)
                                                >
                                                    {if is_locked { "Unlock" } else { "Lock" }}
                                                </button>
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                </div>
            </div>

            // Payment Defaults
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

            // Enabled Networks
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Enabled Networks"</h3>
                </div>
                <div class="detail-card-body">
                    <p class="form-help" style="margin-bottom: 16px;">
                        "Networks available for payment processing. Disabled networks cannot be used by any store."
                    </p>
                    <div class="admin-networks-grid">
                        {all_networks.into_iter().map(|(chain_id, name)| {
                            view! {
                                <div class=move || if enabled_chain_ids.get().contains(&chain_id) { "admin-network-item enabled" } else { "admin-network-item" }>
                                    <div class="admin-network-info">
                                        <span class="admin-network-name">{name}</span>
                                        <span class="admin-network-chain-id">"Chain ID: "{chain_id}</span>
                                    </div>
                                    <label class="toggle">
                                        <input
                                            type="checkbox"
                                            prop:checked=move || enabled_chain_ids.get().contains(&chain_id)
                                            on:change=move |_| toggle_network(chain_id)
                                        />
                                        <span class="toggle-slider"></span>
                                    </label>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </div>
            </div>

            // Maintenance
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

            // Save + status
            <div class="form-actions">
                {move || {
                    let status = settings_status.get();
                    if status.is_empty() {
                        view! { <span></span> }.into_any()
                    } else {
                        view! { <span class="form-status">{status}</span> }.into_any()
                    }
                }}
                <button class="btn btn-primary" on:click=save_settings>"Save server settings"</button>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_date_with_time() {
        assert_eq!(format_date("2026-04-04T12:30:00Z"), "2026-04-04");
    }

    #[test]
    fn test_format_date_date_only() {
        assert_eq!(format_date("2026-04-04"), "2026-04-04");
    }

    #[test]
    fn test_format_date_with_offset() {
        assert_eq!(format_date("2026-04-04T12:30:00+05:00"), "2026-04-04");
    }

    #[test]
    fn test_format_date_empty_string() {
        assert_eq!(format_date(""), "");
    }
}
