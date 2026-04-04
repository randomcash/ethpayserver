//! EVM token definitions and operations.
//!
//! This module provides:
//! - `EvmTokenStandard`: ERC20, ERC721, ERC1155 token standards
//! - `Token`: ERC20 token type and contract interaction functions
//!
//! Token storage/registry is handled by the data-service crate using the generic
//! `TokenData` type from the `types` crate.

use crate::error::{EvmError, EvmResult};
use crate::network::EvmNetwork;
use crate::provider::EvmProvider;
use alloy::primitives::{Address, U256};
use alloy::sol;
use serde::{Deserialize, Serialize};

// =============================================================================
// EVM Token Standards
// =============================================================================

/// Supported token standards on EVM chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvmTokenStandard {
    /// ERC20 - Fungible tokens with decimals.
    Erc20,
    /// ERC721 - Non-fungible tokens (NFTs), each token is unique.
    Erc721,
    /// ERC1155 - Multi-token standard, supports both fungible and non-fungible.
    Erc1155,
}

impl EvmTokenStandard {
    pub fn is_fungible(&self) -> bool {
        matches!(self, Self::Erc20 | Self::Erc1155)
    }

    pub fn is_non_fungible(&self) -> bool {
        matches!(self, Self::Erc721 | Self::Erc1155)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Erc20 => "erc20",
            Self::Erc721 => "erc721",
            Self::Erc1155 => "erc1155",
        }
    }

    /// Validate token data for this standard.
    pub fn validate(
        &self,
        decimals: Option<u8>,
        token_id: Option<&str>,
    ) -> Result<(), &'static str> {
        match self {
            Self::Erc20 => {
                if decimals.is_none() {
                    return Err("ERC20 tokens must have decimals");
                }
                if token_id.is_some() {
                    return Err("ERC20 tokens should not have token_id");
                }
            }
            Self::Erc721 => {
                if decimals.is_some() {
                    return Err("ERC721 tokens should not have decimals");
                }
            }
            Self::Erc1155 => {
                if token_id.is_none() {
                    return Err("ERC1155 tokens must have a token_id");
                }
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for EvmTokenStandard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for EvmTokenStandard {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "erc20" => Ok(Self::Erc20),
            "erc721" => Ok(Self::Erc721),
            "erc1155" => Ok(Self::Erc1155),
            _ => Err(format!("unknown token standard: {}", s)),
        }
    }
}

// =============================================================================
// ERC20 Token Type
// =============================================================================

// Define the ERC20 interface using alloy's sol! macro
sol! {
    #[sol(rpc)]
    interface IERC20 {
        function name() external view returns (string);
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);

        event Transfer(address indexed from, address indexed to, uint256 value);
        event Approval(address indexed owner, address indexed spender, uint256 value);
    }
}

/// ERC20 token definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    /// Database ID (None if not persisted yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Token symbol (e.g., "USDT", "USDC").
    pub symbol: String,
    /// Token name (e.g., "Tether USD").
    pub name: String,
    /// Contract address (checksummed).
    pub address: String,
    /// Number of decimals.
    pub decimals: u8,
    /// Network this token is on.
    pub network: EvmNetwork,
    /// Whether this token is enabled for payments.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Token {
    /// Create a new token.
    pub fn new(
        symbol: impl Into<String>,
        name: impl Into<String>,
        address: impl Into<String>,
        decimals: u8,
        network: EvmNetwork,
    ) -> Self {
        Self {
            id: None,
            symbol: symbol.into(),
            name: name.into(),
            address: address.into(),
            decimals,
            network,
            enabled: true,
        }
    }

    /// Parse the contract address.
    pub fn contract_address(&self) -> EvmResult<Address> {
        self.address
            .parse()
            .map_err(|e| EvmError::InvalidAddress(format!("{}: {}", self.address, e)))
    }

    /// Validate the token address format.
    pub fn validate(&self) -> EvmResult<()> {
        self.contract_address()?;
        Ok(())
    }
}

// =============================================================================
// ERC20 Operations
// =============================================================================

/// Get the ERC20 token balance for an address.
pub async fn get_token_balance(
    provider: &EvmProvider,
    token: &Token,
    address: Address,
) -> EvmResult<U256> {
    let contract_address = token.contract_address()?;
    let contract = IERC20::new(contract_address, provider.inner().clone());

    let result = contract
        .balanceOf(address)
        .call()
        .await
        .map_err(|e| EvmError::Contract(e.to_string()))?;

    Ok(result)
}

/// Get token info from the contract (for discovering unknown tokens).
pub async fn get_token_info(
    provider: &EvmProvider,
    contract_address: Address,
) -> EvmResult<(String, String, u8)> {
    let contract = IERC20::new(contract_address, provider.inner().clone());

    let name = contract
        .name()
        .call()
        .await
        .map_err(|e| EvmError::Contract(format!("failed to get name: {}", e)))?;

    let symbol = contract
        .symbol()
        .call()
        .await
        .map_err(|e| EvmError::Contract(format!("failed to get symbol: {}", e)))?;

    let decimals = contract
        .decimals()
        .call()
        .await
        .map_err(|e| EvmError::Contract(format!("failed to get decimals: {}", e)))?;

    Ok((name, symbol, decimals))
}

/// Discover and create a Token from on-chain data.
pub async fn discover_token(provider: &EvmProvider, contract_address: Address) -> EvmResult<Token> {
    let (name, symbol, decimals) = get_token_info(provider, contract_address).await?;
    let network = provider.chain_config().network.ok_or_else(|| {
        EvmError::Contract("Token discovery not supported on testnets".to_string())
    })?;

    Ok(Token::new(
        symbol,
        name,
        format!("{:?}", contract_address),
        decimals,
        network,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_new() {
        let token = Token::new(
            "USDT",
            "Tether USD",
            "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            6,
            EvmNetwork::Ethereum,
        );

        assert_eq!(token.symbol, "USDT");
        assert_eq!(token.decimals, 6);
        assert!(token.enabled);
        assert!(token.id.is_none());
    }

    #[test]
    fn test_token_contract_address() {
        let token = Token::new(
            "USDT",
            "Tether USD",
            "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            6,
            EvmNetwork::Ethereum,
        );

        let address = token.contract_address().unwrap();
        assert_eq!(
            format!("{:?}", address).to_lowercase(),
            "0xdac17f958d2ee523a2206206994597c13d831ec7"
        );
    }

    #[test]
    fn test_token_validate_invalid_address() {
        let token = Token::new(
            "BAD",
            "Bad Token",
            "not-a-valid-address",
            18,
            EvmNetwork::Ethereum,
        );

        assert!(token.validate().is_err());
    }

    #[test]
    fn test_token_validate_valid_address() {
        let token = Token::new(
            "USDT",
            "Tether USD",
            "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            6,
            EvmNetwork::Ethereum,
        );

        assert!(token.validate().is_ok());
    }
}
