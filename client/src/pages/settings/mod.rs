//! Settings page - Stripe-inspired design with multiple tabs.
//!
//! Contains user settings, preferences, API keys, and admin-only server settings.

mod account;
mod admin;
mod api_keys;
mod notifications;
mod preferences;

use account::AccountTab;
use admin::AdminTab;
use api_keys::ApiKeysTab;
use notifications::NotificationsTab;
use preferences::PreferencesTab;

use crate::api::EvmApiClient;
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
pub(super) fn format_date(iso: &str) -> String {
    if let Some(date_part) = iso.split('T').next() {
        date_part.to_string()
    } else {
        iso.to_string()
    }
}

// ============================================
// Icons
// ============================================

#[component]
pub(super) fn IconShield() -> impl IntoView {
    view! {
        <svg class="settings-tab-icon" xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
        </svg>
    }
}

#[component]
pub(super) fn IconPlus() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}

#[component]
pub(super) fn IconCopy() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
        </svg>
    }
}

#[component]
pub(super) fn IconInfo() -> impl IntoView {
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
