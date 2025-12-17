//! Token repository traits.
//!
//! This module provides generic token storage traits that work across all payserver
//! implementations. Token types and validation are network-specific and should be
//! defined in each payserver's crate (e.g., ERC20/ERC721/ERC1155 in evm crate).

use async_trait::async_trait;

use super::RepositoryResult;
use crate::types::Network;

/// Token data for persistence.
///
/// This is a generic token structure that can represent tokens from any blockchain.
/// Network-specific validation and token types (like ERC20, ERC721, etc.) should
/// be handled in the respective payserver crates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TokenData {
    /// Database ID (None if not persisted yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,

    /// Token type/standard as a string (e.g., "erc20", "erc721", "brc20", "runes").
    /// Each payserver defines its own valid token types.
    pub token_type: String,

    /// Contract/token address or identifier.
    /// Format depends on the network (e.g., "0x..." for EVM, inscription ID for ordinals).
    pub address: String,

    /// Network this token is on.
    pub network: Network,

    /// Whether this token is enabled for payments.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Token name (e.g., "Tether USD", "Bored Ape Yacht Club").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Token symbol (e.g., "USDT", "BAYC").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,

    /// Number of decimals for fungible tokens.
    /// None for non-fungible tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,

    /// Specific token ID within a collection/contract.
    /// Used for NFTs or multi-token standards.
    /// Note: Token IDs can be very large, stored as string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl TokenData {
    /// Create a new token with the basic required fields.
    pub fn new(
        token_type: impl Into<String>,
        address: impl Into<String>,
        network: Network,
    ) -> Self {
        Self {
            id: None,
            token_type: token_type.into(),
            address: address.into(),
            network,
            enabled: true,
            name: None,
            symbol: None,
            decimals: None,
            token_id: None,
        }
    }

    /// Set the token name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the token symbol.
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// Set the number of decimals.
    pub fn with_decimals(mut self, decimals: u8) -> Self {
        self.decimals = Some(decimals);
        self
    }

    /// Set the token ID (for NFTs or multi-token standards).
    pub fn with_token_id(mut self, token_id: impl Into<String>) -> Self {
        self.token_id = Some(token_id.into());
        self
    }

    /// Set whether this token is enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Query parameters for listing tokens.
#[derive(Debug, Clone)]
pub struct TokenQueryParams {
    pub token_type: Option<String>,
    pub network: Option<Network>,
    pub enabled: Option<bool>,
    pub symbol: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

impl Default for TokenQueryParams {
    fn default() -> Self {
        Self {
            token_type: None,
            network: None,
            enabled: None,
            symbol: None,
            limit: 100,
            offset: 0,
        }
    }
}

impl TokenQueryParams {
    pub fn with_token_type(mut self, token_type: impl Into<String>) -> Self {
        self.token_type = Some(token_type.into());
        self
    }

    pub fn with_network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_offset(mut self, offset: i64) -> Self {
        self.offset = offset;
        self
    }
}

/// Read operations for tokens.
#[async_trait]
pub trait TokenReader: Send + Sync {
    /// Get a token by ID.
    async fn get(&self, id: i64) -> RepositoryResult<Option<TokenData>>;

    /// Get a token by network and contract address.
    async fn get_by_address(&self, network: Network, address: &str) -> RepositoryResult<Option<TokenData>>;

    /// Find a token by network and symbol.
    /// Note: If multiple tokens have the same symbol on a network, returns the first match.
    async fn find_by_symbol(&self, network: Network, symbol: &str) -> RepositoryResult<Option<TokenData>>;

    /// Query tokens with filters and pagination.
    /// Returns (total_count, tokens).
    async fn query(&self, params: &TokenQueryParams) -> RepositoryResult<(i64, Vec<TokenData>)>;

    /// Get all enabled tokens for a specific network.
    async fn get_enabled_for_network(&self, network: Network) -> RepositoryResult<Vec<TokenData>>;
}

/// Write operations for tokens.
#[async_trait]
pub trait TokenWriter: Send + Sync {
    /// Insert a new token. Returns the assigned ID.
    async fn insert(&self, token: &TokenData) -> RepositoryResult<i64>;

    /// Update an existing token.
    async fn update(&self, token: &TokenData) -> RepositoryResult<()>;

    /// Delete a token by ID.
    async fn delete(&self, id: i64) -> RepositoryResult<()>;

    /// Enable or disable a token.
    async fn set_enabled(&self, id: i64, enabled: bool) -> RepositoryResult<()>;
}

/// Combined token repository with full read/write access.
pub trait TokenRepository: TokenReader + TokenWriter {}

/// Blanket implementation for any type implementing both Reader and Writer.
impl<T: TokenReader + TokenWriter> TokenRepository for T {}
