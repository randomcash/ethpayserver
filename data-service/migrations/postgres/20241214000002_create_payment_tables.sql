-- Payment tables migration for EthPayServer
-- Supports EVM-compatible networks only

-- =============================================================================
-- Enums
-- =============================================================================

-- Supported EVM networks
CREATE TYPE network AS ENUM (
    'ethereum',
    'polygon',
    'arbitrum',
    'optimism',
    'base',
    'avalanche',
    'binance_smart_chain',
    'zksync',
    'linea',
    'scroll'
);

-- Asset type (native or ERC20 token)
CREATE TYPE asset_type AS ENUM (
    'native',        -- Native network currency (ETH, POL, AVAX, BNB)
    'erc20'          -- ERC20 token
);

-- Invoice status
CREATE TYPE invoice_status AS ENUM (
    'pending',
    'processing',
    'partially_paid',
    'paid',
    'expired',
    'cancelled',
    'refunded',
    'late_paid'
);

-- =============================================================================
-- Tokens table
-- Generic token storage for all payservers.
-- Token type validation is done at the application layer, not in the database,
-- to support different token standards across different blockchain networks.
-- =============================================================================
CREATE TABLE tokens (
    id BIGSERIAL PRIMARY KEY,

    -- Token type (e.g., "erc20", "erc721", "erc1155", "brc20", "runes")
    -- Validation of valid token types is done at the application layer
    token_type VARCHAR(32) NOT NULL,

    -- Contract/token address or identifier
    -- Format depends on the network (checksummed for EVM)
    address VARCHAR(128) NOT NULL,

    -- Network (matches network enum for EVM, or string for other chains)
    network VARCHAR(32) NOT NULL,

    -- Status
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Metadata
    name VARCHAR(128),
    symbol VARCHAR(32),

    -- Number of decimals for fungible tokens
    decimals SMALLINT CHECK (decimals IS NULL OR (decimals >= 0 AND decimals <= 18)),

    -- Specific token ID within a collection/contract (for NFTs)
    token_id VARCHAR(78),  -- uint256 max is 78 digits

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique constraint: network + address + token_id (token_id can be NULL)
    CONSTRAINT tokens_unique UNIQUE NULLS NOT DISTINCT (network, address, token_id)
);

-- Indexes for tokens
CREATE INDEX idx_tokens_token_type ON tokens(token_type);
CREATE INDEX idx_tokens_network ON tokens(network);
CREATE INDEX idx_tokens_network_enabled ON tokens(network, enabled) WHERE enabled = true;
CREATE INDEX idx_tokens_symbol ON tokens(LOWER(symbol)) WHERE symbol IS NOT NULL;
CREATE INDEX idx_tokens_address ON tokens(LOWER(address));
CREATE INDEX idx_tokens_type_network ON tokens(token_type, network);

-- Trigger to update updated_at on tokens
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

-- =============================================================================
-- Invoices table
-- =============================================================================
CREATE TABLE invoices (
    id VARCHAR(64) PRIMARY KEY,  -- UUID string

    -- Requested amount (in smallest unit: wei for native, token decimals for ERC20)
    amount_value NUMERIC(78, 0) NOT NULL,  -- uint256 max is 78 digits

    -- Asset specification
    network network NOT NULL,
    asset_type asset_type NOT NULL,
    token_id BIGINT REFERENCES tokens(id),   -- NULL for native assets

    -- Asset symbol for display (e.g., 'ETH', 'POL', 'USDT')
    asset_symbol VARCHAR(32) NOT NULL,

    -- Payment details
    payment_address VARCHAR(42),           -- Checksummed EVM address

    -- Status
    status invoice_status NOT NULL DEFAULT 'pending',
    amount_received NUMERIC(78, 0) NOT NULL DEFAULT 0,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,

    -- Optional metadata (encrypted by client if sensitive)
    metadata JSONB,

    -- Webhook and redirect URLs
    webhook_url TEXT,
    redirect_url TEXT,

    -- Link to store (required - invoices belong to stores)
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,

    -- Chain-specific extra data
    extra JSONB,

    -- Ensure ERC20 invoices have a token reference
    CONSTRAINT invoice_token_required CHECK (
        (asset_type = 'native' AND token_id IS NULL) OR
        (asset_type = 'erc20' AND token_id IS NOT NULL)
    )
);

-- Indexes for invoices
CREATE INDEX idx_invoices_status ON invoices(status);
CREATE INDEX idx_invoices_network ON invoices(network);
CREATE INDEX idx_invoices_store_id ON invoices(store_id);
CREATE INDEX idx_invoices_expires_at ON invoices(expires_at);
CREATE INDEX idx_invoices_created_at ON invoices(created_at);
CREATE INDEX idx_invoices_payment_address ON invoices(payment_address) WHERE payment_address IS NOT NULL;
CREATE INDEX idx_invoices_pending ON invoices(status, expires_at)
    WHERE status IN ('pending', 'processing', 'partially_paid');

-- =============================================================================
-- Payments table
-- =============================================================================
CREATE TABLE payments (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    invoice_id VARCHAR(64) NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,

    -- Amount received (in smallest unit)
    amount_value NUMERIC(78, 0) NOT NULL,

    -- Asset (should match invoice, but stored for audit)
    network network NOT NULL,
    asset_type asset_type NOT NULL,
    token_id BIGINT REFERENCES tokens(id),
    asset_symbol VARCHAR(32) NOT NULL,

    -- Transaction details
    tx_hash VARCHAR(66) NOT NULL,          -- 0x + 64 hex chars
    block_number BIGINT,
    confirmations INTEGER NOT NULL DEFAULT 0,
    from_address VARCHAR(42),              -- Checksummed EVM address

    -- Timestamps
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at TIMESTAMPTZ,

    -- Reorg handling
    reorged BOOLEAN NOT NULL DEFAULT FALSE,

    -- Chain-specific extra data
    extra JSONB,

    CONSTRAINT unique_payment_tx UNIQUE (tx_hash, network)
);

-- Indexes for payments
CREATE INDEX idx_payments_invoice_id ON payments(invoice_id);
CREATE INDEX idx_payments_tx_hash ON payments(tx_hash);
CREATE INDEX idx_payments_network ON payments(network);
CREATE INDEX idx_payments_confirmations ON payments(confirmations);
CREATE INDEX idx_payments_detected_at ON payments(detected_at);

-- =============================================================================
-- Watched addresses table (for payment monitoring)
-- =============================================================================
CREATE TABLE watched_addresses (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    invoice_id VARCHAR(64) NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,

    -- Address info
    address VARCHAR(42) NOT NULL,          -- Checksummed EVM address
    network network NOT NULL,
    asset_type asset_type NOT NULL,
    token_address VARCHAR(42),             -- For ERC20 tokens

    -- Monitoring state
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    last_checked_block BIGINT,
    monitor_notified BOOLEAN NOT NULL DEFAULT FALSE,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,

    CONSTRAINT unique_watched_address UNIQUE (address, network, token_address)
);

-- Indexes for watched_addresses
CREATE INDEX idx_watched_active ON watched_addresses(is_active, network) WHERE is_active = TRUE;
CREATE INDEX idx_watched_invoice ON watched_addresses(invoice_id);
CREATE INDEX idx_watched_address ON watched_addresses(address, network);
CREATE INDEX idx_watched_expires ON watched_addresses(expires_at);
CREATE INDEX idx_watched_pending ON watched_addresses(monitor_notified, is_active)
    WHERE monitor_notified = FALSE AND is_active = TRUE;

-- =============================================================================
-- Payment events table (audit log)
-- =============================================================================
CREATE TABLE payment_events (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    invoice_id VARCHAR(64) NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    payment_id UUID REFERENCES payments(id) ON DELETE SET NULL,

    -- Event info
    event_type VARCHAR(64) NOT NULL,  -- 'created', 'detected', 'confirmed', 'status_changed', etc.
    event_data JSONB,

    -- Timestamp
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for payment_events
CREATE INDEX idx_payment_events_invoice ON payment_events(invoice_id);
CREATE INDEX idx_payment_events_type ON payment_events(event_type);
CREATE INDEX idx_payment_events_created ON payment_events(created_at);

-- =============================================================================
-- Network configuration table
-- Stores RPC endpoints, confirmation requirements, etc. per network
-- =============================================================================
CREATE TABLE network_configs (
    network network PRIMARY KEY,

    -- Network metadata
    chain_id INTEGER NOT NULL,             -- EIP-155 chain ID
    name VARCHAR(64) NOT NULL,
    native_symbol VARCHAR(16) NOT NULL,    -- ETH, POL, AVAX, BNB
    native_decimals SMALLINT NOT NULL DEFAULT 18,

    -- Block explorer
    explorer_url TEXT,
    explorer_tx_path VARCHAR(64) DEFAULT '/tx/',
    explorer_address_path VARCHAR(64) DEFAULT '/address/',

    -- Confirmation requirements
    required_confirmations INTEGER NOT NULL DEFAULT 6,

    -- RPC configuration (can be overridden in app config)
    default_rpc_url TEXT,

    -- Status
    is_active BOOLEAN NOT NULL DEFAULT TRUE,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- =============================================================================
-- Seed default network configurations
-- =============================================================================
INSERT INTO network_configs (network, chain_id, name, native_symbol, native_decimals, explorer_url, required_confirmations) VALUES
    ('ethereum', 1, 'Ethereum', 'ETH', 18, 'https://etherscan.io', 12),
    ('polygon', 137, 'Polygon', 'POL', 18, 'https://polygonscan.com', 128),
    ('arbitrum', 42161, 'Arbitrum', 'ETH', 18, 'https://arbiscan.io', 12),
    ('optimism', 10, 'Optimism', 'ETH', 18, 'https://optimistic.etherscan.io', 12),
    ('base', 8453, 'Base', 'ETH', 18, 'https://basescan.org', 12),
    ('avalanche', 43114, 'Avalanche', 'AVAX', 18, 'https://snowtrace.io', 12),
    ('binance_smart_chain', 56, 'BNB Chain', 'BNB', 18, 'https://bscscan.com', 15),
    ('zksync', 324, 'zkSync', 'ETH', 18, 'https://explorer.zksync.io', 1),
    ('linea', 59144, 'Linea', 'ETH', 18, 'https://lineascan.build', 12),
    ('scroll', 534352, 'Scroll', 'ETH', 18, 'https://scrollscan.com', 12);

-- =============================================================================
-- Seed common tokens (USDT, USDC) on major EVM networks
-- =============================================================================
INSERT INTO tokens (token_type, network, address, symbol, name, decimals) VALUES
    -- Ethereum
    ('erc20', 'ethereum', '0xdAC17F958D2ee523a2206206994597C13D831ec7', 'USDT', 'Tether USD', 6),
    ('erc20', 'ethereum', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', 'USDC', 'USD Coin', 6),
    ('erc20', 'ethereum', '0x6B175474E89094C44Da98b954EedeAC495271d0F', 'DAI', 'Dai Stablecoin', 18),
    -- Polygon
    ('erc20', 'polygon', '0xc2132D05D31c914a87C6611C10748AEb04B58e8F', 'USDT', 'Tether USD', 6),
    ('erc20', 'polygon', '0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359', 'USDC', 'USD Coin', 6),
    -- Arbitrum
    ('erc20', 'arbitrum', '0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9', 'USDT', 'Tether USD', 6),
    ('erc20', 'arbitrum', '0xaf88d065e77c8cC2239327C5EDb3A432268e5831', 'USDC', 'USD Coin', 6),
    -- Optimism
    ('erc20', 'optimism', '0x94b008aA00579c1307B0EF2c499aD98a8ce58e58', 'USDT', 'Tether USD', 6),
    ('erc20', 'optimism', '0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85', 'USDC', 'USD Coin', 6),
    -- Base
    ('erc20', 'base', '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913', 'USDC', 'USD Coin', 6),
    -- BNB Smart Chain
    ('erc20', 'binance_smart_chain', '0x55d398326f99059fF775485246999027B3197955', 'USDT', 'Tether USD', 18),
    ('erc20', 'binance_smart_chain', '0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d', 'USDC', 'USD Coin', 18),
    -- Avalanche
    ('erc20', 'avalanche', '0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7', 'USDT', 'Tether USD', 6),
    ('erc20', 'avalanche', '0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E', 'USDC', 'USD Coin', 6);

-- =============================================================================
-- Helper functions
-- =============================================================================

-- Function to update invoice amount_received and set status to processing when payment is detected
-- Note: Status transitions to 'paid' are handled by the application after confirmations are verified
-- Reorged payments are excluded from amount_received calculation
CREATE OR REPLACE FUNCTION update_invoice_on_payment()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE invoices
    SET amount_received = (
        SELECT COALESCE(SUM(amount_value), 0)
        FROM payments
        WHERE invoice_id = NEW.invoice_id AND reorged = FALSE
    ),
    -- Only transition pending -> processing; paid is set by app after confirmations
    status = CASE
        WHEN status = 'pending' AND NEW.reorged = FALSE THEN 'processing'::invoice_status
        ELSE status
    END
    WHERE id = NEW.invoice_id;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to update invoice when payment is inserted or updated
CREATE TRIGGER trigger_update_invoice_on_payment
AFTER INSERT OR UPDATE ON payments
FOR EACH ROW
EXECUTE FUNCTION update_invoice_on_payment();

-- Function to expire old invoices
CREATE OR REPLACE FUNCTION expire_old_invoices()
RETURNS INTEGER AS $$
DECLARE
    expired_count INTEGER;
BEGIN
    UPDATE invoices
    SET status = 'expired'
    WHERE status IN ('pending', 'processing')
    AND expires_at < NOW();

    GET DIAGNOSTICS expired_count = ROW_COUNT;
    RETURN expired_count;
END;
$$ LANGUAGE plpgsql;

-- Function to get asset display string (e.g., 'ETH', 'ETH-ERC20-USDT')
CREATE OR REPLACE FUNCTION get_asset_display(
    p_network network,
    p_asset_type asset_type,
    p_token_id BIGINT
) RETURNS TEXT AS $$
DECLARE
    network_symbol TEXT;
    token_symbol TEXT;
BEGIN
    -- Get native symbol for network
    SELECT native_symbol INTO network_symbol FROM network_configs WHERE network = p_network;

    IF p_asset_type = 'native' THEN
        RETURN network_symbol;
    ELSE
        -- Get token symbol
        SELECT symbol INTO token_symbol FROM tokens WHERE id = p_token_id;
        RETURN network_symbol || '-ERC20-' || token_symbol;
    END IF;
END;
$$ LANGUAGE plpgsql;

-- =============================================================================
-- Comments for documentation
-- =============================================================================
COMMENT ON TABLE tokens IS 'Registry of known ERC20 tokens across EVM networks';
COMMENT ON TABLE invoices IS 'Payment invoices for EVM networks';
COMMENT ON TABLE payments IS 'Received payments linked to invoices';
COMMENT ON TABLE watched_addresses IS 'Addresses being monitored for incoming payments';
COMMENT ON TABLE payment_events IS 'Audit log of payment-related events';
COMMENT ON TABLE network_configs IS 'Configuration for supported EVM networks';

COMMENT ON COLUMN invoices.amount_value IS 'Amount in smallest unit (wei for native, token decimals for ERC20)';
COMMENT ON COLUMN invoices.metadata IS 'Optional JSON metadata (may be encrypted by client)';
COMMENT ON COLUMN payments.tx_hash IS 'Transaction hash (0x + 64 hex chars)';

COMMENT ON TYPE network IS 'Supported EVM-compatible networks';
COMMENT ON TYPE asset_type IS 'Asset type: native currency or ERC20 token';
