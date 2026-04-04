//! Token selector component for choosing ERC20 tokens.

use leptos::prelude::*;

/// Common ERC20 tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
    /// Native token (ETH, MATIC, etc.) has no address.
    pub address: Option<&'static str>,
}

impl Token {
    pub const fn native(symbol: &'static str, name: &'static str) -> Self {
        Self {
            symbol,
            name,
            decimals: 18,
            address: None,
        }
    }

    pub const fn erc20(
        symbol: &'static str,
        name: &'static str,
        decimals: u8,
        address: &'static str,
    ) -> Self {
        Self {
            symbol,
            name,
            decimals,
            address: Some(address),
        }
    }
}

/// Common tokens available on most EVM chains.
pub const COMMON_TOKENS: &[Token] = &[
    Token::native("ETH", "Ether"),
    Token::erc20(
        "USDC",
        "USD Coin",
        6,
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    ),
    Token::erc20(
        "USDT",
        "Tether USD",
        6,
        "0xdAC17F958D2ee523a2206206994597C13D831ec7",
    ),
    Token::erc20(
        "DAI",
        "Dai Stablecoin",
        18,
        "0x6B175474E89094C44Da98b954EescdeCB5BE00",
    ),
    Token::erc20(
        "WETH",
        "Wrapped Ether",
        18,
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
    ),
];

/// Token selector component.
#[component]
pub fn TokenSelector(
    /// Currently selected token symbol.
    selected: RwSignal<String>,
    /// Available tokens (defaults to common tokens).
    #[prop(optional)]
    tokens: Option<Vec<Token>>,
    /// Callback when selection changes.
    #[prop(optional)]
    on_change: Option<Callback<String>>,
) -> impl IntoView {
    let token_list = tokens.unwrap_or_else(|| COMMON_TOKENS.to_vec());

    view! {
        <div class="evm-token-selector">
            {token_list.into_iter().map(|token| {
                let symbol = token.symbol.to_string();
                let symbol_for_click = symbol.clone();
                let is_selected = {
                    let symbol = symbol.clone();
                    move || selected.get() == symbol
                };

                view! {
                    <button
                        class="evm-token-option"
                        class:selected=is_selected
                        on:click=move |_| {
                            selected.set(symbol_for_click.clone());
                            if let Some(cb) = &on_change {
                                cb.run(symbol_for_click.clone());
                            }
                        }
                    >
                        <span class="evm-token-symbol">{token.symbol}</span>
                        <span class="evm-token-name">{token.name}</span>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_token_creation() {
        let token = Token::native("ETH", "Ether");
        assert_eq!(token.symbol, "ETH");
        assert_eq!(token.name, "Ether");
        assert_eq!(token.decimals, 18);
        assert!(token.address.is_none());
    }

    #[test]
    fn test_erc20_token_creation() {
        let token = Token::erc20("USDC", "USD Coin", 6, "0xA0b86991...");
        assert_eq!(token.symbol, "USDC");
        assert_eq!(token.name, "USD Coin");
        assert_eq!(token.decimals, 6);
        assert!(token.address.is_some());
    }

    #[test]
    fn test_common_tokens_list() {
        assert!(!COMMON_TOKENS.is_empty());
        // First token should be ETH (native)
        assert_eq!(COMMON_TOKENS[0].symbol, "ETH");
        assert!(COMMON_TOKENS[0].address.is_none());
        // USDC should have 6 decimals
        let usdc = COMMON_TOKENS.iter().find(|t| t.symbol == "USDC").unwrap();
        assert_eq!(usdc.decimals, 6);
    }

    #[test]
    fn test_token_equality() {
        let token1 = Token::native("ETH", "Ether");
        let token2 = Token::native("ETH", "Ether");
        assert_eq!(token1, token2);
    }
}

/// Token selector styles.
#[allow(dead_code)]
pub const TOKEN_SELECTOR_STYLES: &str = r#"
.evm-token-selector {
    display: flex;
    flex-direction: column;
    gap: var(--ps-spacing-xs);
}

.evm-token-option {
    display: flex;
    align-items: center;
    gap: var(--ps-spacing-sm);
    padding: var(--ps-spacing-sm) var(--ps-spacing-md);
    background: transparent;
    border: 1px solid var(--ps-border);
    border-radius: var(--ps-radius-md);
    cursor: pointer;
    transition: all 0.15s ease;
    text-align: left;
}

.evm-token-option:hover {
    border-color: var(--ps-primary);
    background-color: var(--ps-surface);
}

.evm-token-option.selected {
    border-color: var(--ps-primary);
    background-color: rgba(37, 99, 235, 0.1);
}

.evm-token-symbol {
    font-weight: 600;
    min-width: 60px;
}

.evm-token-name {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}
"#;
