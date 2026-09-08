//! Network selector component for choosing EVM networks.

use leptos::prelude::*;
use types::ChainId;
use ui_kit::components::crypto::NetworkBadge;

/// EVM chains offered by this server, as (display name, EIP-155 id).
///
/// The name is carried alongside because a CAIP-2 identifier does not imply
/// one - `eip155:137` is not "Polygon" to anything but a lookup. A multi-chain
/// shell drives this from `chain_configs`; this server knows its own chains.
const AVAILABLE_NETWORKS: &[(&str, u64)] = &[
    ("Ethereum", 1),
    ("Polygon", 137),
    ("Arbitrum", 42161),
    ("Optimism", 10),
    ("Base", 8453),
    ("Avalanche", 43114),
    ("BNB Chain", 56),
    ("zkSync", 324),
    ("Linea", 59144),
    ("Scroll", 534352),
    ("Fantom", 250),
    ("Gnosis", 100),
];

/// Network selector component.
#[component]
pub fn NetworkSelector(
    /// Currently selected chain ID.
    selected: RwSignal<Option<u64>>,
    /// Callback when selection changes.
    #[prop(optional)]
    on_change: Option<Callback<Option<u64>>>,
    /// Whether multiple selection is allowed.
    #[prop(default = false)]
    multi: bool,
    /// Multiple selected chain IDs (for multi mode).
    #[prop(optional)]
    selected_multi: Option<RwSignal<Vec<u64>>>,
) -> impl IntoView {
    if multi {
        view! { <MultiNetworkSelector selected=selected_multi.unwrap_or_else(|| RwSignal::new(vec![])) /> }.into_any()
    } else {
        view! { <SingleNetworkSelector selected=selected on_change=on_change /> }.into_any()
    }
}

#[component]
fn SingleNetworkSelector(
    selected: RwSignal<Option<u64>>,
    on_change: Option<Callback<Option<u64>>>,
) -> impl IntoView {
    view! {
        <div class="evm-network-selector">
            {AVAILABLE_NETWORKS.iter().map(|(name, chain_id)| {
                let chain_id = *chain_id;
                let name = *name;
                let is_selected = move || selected.get() == Some(chain_id);

                view! {
                    <button
                        class="evm-network-option"
                        class:selected=is_selected
                        on:click=move |_| {
                            let new_value = if is_selected() { None } else { Some(chain_id) };
                            selected.set(new_value);
                            if let Some(cb) = &on_change {
                                cb.run(new_value);
                            }
                        }
                    >
                        <NetworkBadge chain_id=ChainId::evm(chain_id) name=name.to_string() />
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn MultiNetworkSelector(selected: RwSignal<Vec<u64>>) -> impl IntoView {
    view! {
        <div class="evm-network-selector evm-network-selector-multi">
            {AVAILABLE_NETWORKS.iter().map(|(name, chain_id)| {
                let chain_id = *chain_id;
                let name = *name;
                let is_selected = move || selected.get().contains(&chain_id);

                view! {
                    <button
                        class="evm-network-option"
                        class:selected=is_selected
                        on:click=move |_| {
                            selected.update(|list| {
                                if let Some(pos) = list.iter().position(|&id| id == chain_id) {
                                    list.remove(pos);
                                } else {
                                    list.push(chain_id);
                                }
                            });
                        }
                    >
                        <NetworkBadge chain_id=ChainId::evm(chain_id) name=name.to_string() />
                        {move || if is_selected() {
                            view! { <span class="evm-network-check">"✓"</span> }.into_any()
                        } else {
                            view! { <span></span> }.into_any()
                        }}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

/// Network selector styles.
#[allow(dead_code)]
pub const NETWORK_SELECTOR_STYLES: &str = r#"
.evm-network-selector {
    display: flex;
    flex-wrap: wrap;
    gap: var(--ps-spacing-sm);
}

.evm-network-option {
    display: flex;
    align-items: center;
    gap: var(--ps-spacing-xs);
    padding: var(--ps-spacing-xs) var(--ps-spacing-sm);
    background: transparent;
    border: 1px solid var(--ps-border);
    border-radius: var(--ps-radius-md);
    cursor: pointer;
    transition: all 0.15s ease;
}

.evm-network-option:hover {
    border-color: var(--ps-primary);
}

.evm-network-option.selected {
    border-color: var(--ps-primary);
    background-color: rgba(37, 99, 235, 0.1);
}

.evm-network-check {
    color: var(--ps-primary);
    font-weight: bold;
}
"#;
