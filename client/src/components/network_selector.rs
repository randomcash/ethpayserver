//! Network selector component for choosing EVM networks.

use leptos::prelude::*;
use types::Network;
use ui_kit::components::crypto::NetworkBadge;

/// Available EVM networks for selection.
const AVAILABLE_NETWORKS: &[(Network, u64)] = &[
    (Network::Ethereum, 1),
    (Network::Polygon, 137),
    (Network::Arbitrum, 42161),
    (Network::Optimism, 10),
    (Network::Base, 8453),
    (Network::Avalanche, 43114),
    (Network::BinanceSmartChain, 56),
    (Network::ZkSync, 324),
    (Network::Linea, 59144),
    (Network::Scroll, 534352),
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
            {AVAILABLE_NETWORKS.iter().map(|(network, chain_id)| {
                let chain_id = *chain_id;
                let network = *network;
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
                        <NetworkBadge network=network />
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
            {AVAILABLE_NETWORKS.iter().map(|(network, chain_id)| {
                let chain_id = *chain_id;
                let network = *network;
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
                        <NetworkBadge network=network />
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
