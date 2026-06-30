//! Admin settings tab - server settings and user management (admin only).

use crate::api::{AdminUserInfo, EvmApiClient, UpdateServerSettingsRequest, UpdateUserRoleRequest};
use leptos::prelude::*;

use super::IconShield;

/// Admin tab - server settings and user management (admin only).
#[component]
pub fn AdminTab() -> impl IntoView {
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
