//! Gas fee estimator component.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use ui_kit::components::Skeleton;

/// Gas price data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GasPrice {
    pub slow: u64,
    pub standard: u64,
    pub fast: u64,
    pub instant: u64,
}

impl Default for GasPrice {
    fn default() -> Self {
        Self {
            slow: 10,
            standard: 15,
            fast: 25,
            instant: 40,
        }
    }
}

/// Gas estimator component.
#[component]
pub fn GasEstimator(
    /// Chain ID to fetch gas prices for.
    chain_id: u64,
    /// Gas price in gwei (if known).
    #[prop(optional)] gas_price: Option<GasPrice>,
) -> impl IntoView {
    let prices = Resource::new(
        move || chain_id,
        |_chain_id| async {
            // TODO: Fetch from RPC or gas API
            Ok::<_, String>(GasPrice::default())
        },
    );

    view! {
        <div class="evm-gas-estimator">
            <Suspense fallback=move || view! { <GasEstimatorSkeleton /> }>
                {move || {
                    let price_data = gas_price.clone().or_else(|| prices.get().and_then(|r| r.ok()));
                    price_data.map(|gas| view! { <GasEstimatorContent gas=gas /> })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn GasEstimatorContent(gas: GasPrice) -> impl IntoView {
    view! {
        <div class="evm-gas-options">
            <GasOption label="Slow" gwei=gas.slow time="~5 min" />
            <GasOption label="Standard" gwei=gas.standard time="~2 min" selected=true />
            <GasOption label="Fast" gwei=gas.fast time="~30 sec" />
            <GasOption label="Instant" gwei=gas.instant time="~15 sec" />
        </div>
    }
}

#[component]
fn GasOption(
    label: &'static str,
    gwei: u64,
    time: &'static str,
    #[prop(default = false)] selected: bool,
) -> impl IntoView {
    let class = if selected {
        "evm-gas-option selected"
    } else {
        "evm-gas-option"
    };

    view! {
        <div class=class>
            <span class="evm-gas-label">{label}</span>
            <span class="evm-gas-gwei">{gwei} " gwei"</span>
            <span class="evm-gas-time">{time}</span>
        </div>
    }
}

#[component]
fn GasEstimatorSkeleton() -> impl IntoView {
    view! {
        <div class="evm-gas-options">
            <Skeleton height="60px" />
            <Skeleton height="60px" />
            <Skeleton height="60px" />
            <Skeleton height="60px" />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_price_default() {
        let gas = GasPrice::default();
        assert_eq!(gas.slow, 10);
        assert_eq!(gas.standard, 15);
        assert_eq!(gas.fast, 25);
        assert_eq!(gas.instant, 40);
    }

    #[test]
    fn test_gas_price_ordering() {
        let gas = GasPrice::default();
        assert!(gas.slow < gas.standard);
        assert!(gas.standard < gas.fast);
        assert!(gas.fast < gas.instant);
    }

    #[test]
    fn test_gas_price_clone() {
        let gas1 = GasPrice {
            slow: 5,
            standard: 10,
            fast: 20,
            instant: 50,
        };
        let gas2 = gas1.clone();
        assert_eq!(gas1.slow, gas2.slow);
        assert_eq!(gas1.instant, gas2.instant);
    }
}

/// Gas estimator styles.
pub const GAS_ESTIMATOR_STYLES: &str = r#"
.evm-gas-estimator {
    padding: var(--ps-spacing-sm);
}

.evm-gas-options {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--ps-spacing-sm);
}

.evm-gas-option {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--ps-spacing-xs);
    padding: var(--ps-spacing-md);
    background-color: var(--ps-background);
    border: 1px solid var(--ps-border);
    border-radius: var(--ps-radius-md);
    cursor: pointer;
    transition: all 0.15s ease;
}

.evm-gas-option:hover {
    border-color: var(--ps-primary);
}

.evm-gas-option.selected {
    border-color: var(--ps-primary);
    background-color: rgba(37, 99, 235, 0.1);
}

.evm-gas-label {
    font-weight: 600;
    font-size: var(--ps-font-sm);
}

.evm-gas-gwei {
    font-family: monospace;
    font-size: var(--ps-font-lg);
}

.evm-gas-time {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}
"#;
