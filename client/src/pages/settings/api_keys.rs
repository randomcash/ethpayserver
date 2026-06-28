//! API Keys settings tab.

use crate::api::{
    ApiKeyInfo, CreateApiKeyRequest, CreateApiKeyResponsePayload, EvmApiClient,
    RotateApiKeyResponse,
};
use leptos::prelude::*;

use super::{IconInfo, IconPlus};

/// API Keys tab.
#[component]
pub fn ApiKeysTab() -> impl IntoView {
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
