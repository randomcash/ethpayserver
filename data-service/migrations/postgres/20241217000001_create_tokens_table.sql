-- Tokens table
-- Generic token storage for all payservers.
-- Token type validation is done at the application layer, not in the database,
-- to support different token standards across different blockchain networks.

CREATE TABLE IF NOT EXISTS tokens (
    id BIGSERIAL PRIMARY KEY,

    -- Token type (e.g., "erc20", "erc721", "erc1155", "brc20", "runes")
    -- Validation of valid token types is done at the application layer
    token_type VARCHAR(32) NOT NULL,

    -- Contract/token address or identifier
    -- Format depends on the network
    address VARCHAR(128) NOT NULL,

    -- Network
    network VARCHAR(32) NOT NULL,

    -- Status
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Metadata (optional)
    name VARCHAR(128),
    symbol VARCHAR(32),

    -- Number of decimals for fungible tokens
    decimals SMALLINT CHECK (decimals IS NULL OR (decimals >= 0 AND decimals <= 18)),

    -- Specific token ID within a collection/contract
    -- Used for NFTs or multi-token standards
    token_id VARCHAR(78),  -- uint256 max is 78 digits

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique constraint: network + address + token_id (token_id can be NULL)
    CONSTRAINT tokens_unique UNIQUE NULLS NOT DISTINCT (network, address, token_id)
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_tokens_token_type ON tokens(token_type);
CREATE INDEX IF NOT EXISTS idx_tokens_network ON tokens(network);
CREATE INDEX IF NOT EXISTS idx_tokens_network_enabled ON tokens(network, enabled) WHERE enabled = true;
CREATE INDEX IF NOT EXISTS idx_tokens_symbol ON tokens(LOWER(symbol)) WHERE symbol IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tokens_address ON tokens(LOWER(address));
CREATE INDEX IF NOT EXISTS idx_tokens_type_network ON tokens(token_type, network);

-- Trigger to update updated_at
CREATE OR REPLACE FUNCTION update_tokens_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tokens_updated_at
    BEFORE UPDATE ON tokens
    FOR EACH ROW
    EXECUTE FUNCTION update_tokens_updated_at();

-- Insert default ERC20 tokens (common stablecoins)
INSERT INTO tokens (token_type, symbol, name, address, decimals, network, enabled) VALUES
    -- Ethereum
    ('erc20', 'USDT', 'Tether USD', '0xdAC17F958D2ee523a2206206994597C13D831ec7', 6, 'ethereum', true),
    ('erc20', 'USDC', 'USD Coin', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', 6, 'ethereum', true),
    ('erc20', 'DAI', 'Dai Stablecoin', '0x6B175474E89094C44Da98b954EedeAC495271d0F', 18, 'ethereum', true),

    -- Polygon
    ('erc20', 'USDT', 'Tether USD', '0xc2132D05D31c914a87C6611C10748AEb04B58e8F', 6, 'polygon', true),
    ('erc20', 'USDC', 'USD Coin', '0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359', 6, 'polygon', true),

    -- Arbitrum
    ('erc20', 'USDT', 'Tether USD', '0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9', 6, 'arbitrum', true),
    ('erc20', 'USDC', 'USD Coin', '0xaf88d065e77c8cC2239327C5EDb3A432268e5831', 6, 'arbitrum', true),

    -- Optimism
    ('erc20', 'USDT', 'Tether USD', '0x94b008aA00579c1307B0EF2c499aD98a8ce58e58', 6, 'optimism', true),
    ('erc20', 'USDC', 'USD Coin', '0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85', 6, 'optimism', true),

    -- Base
    ('erc20', 'USDC', 'USD Coin', '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913', 6, 'base', true),

    -- BNB Smart Chain
    ('erc20', 'USDT', 'Tether USD', '0x55d398326f99059fF775485246999027B3197955', 18, 'binance_smart_chain', true),
    ('erc20', 'USDC', 'USD Coin', '0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d', 18, 'binance_smart_chain', true),

    -- Avalanche
    ('erc20', 'USDT', 'Tether USD', '0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7', 6, 'avalanche', true),
    ('erc20', 'USDC', 'USD Coin', '0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E', 6, 'avalanche', true)
ON CONFLICT (network, address, token_id) DO NOTHING;
