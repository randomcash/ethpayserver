-- Payment tables migration for EthPayServer
-- Supports EVM-compatible networks with network-agnostic invoices

-- =============================================================================
-- Enums
-- =============================================================================

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

    -- Chain ID (EIP-155)
    chain_id BIGINT NOT NULL,

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

    -- Unique constraint: chain_id + address + token_id (token_id can be NULL)
    CONSTRAINT tokens_unique UNIQUE NULLS NOT DISTINCT (chain_id, address, token_id)
);

-- Indexes for tokens
CREATE INDEX idx_tokens_token_type ON tokens(token_type);
CREATE INDEX idx_tokens_chain_id ON tokens(chain_id);
CREATE INDEX idx_tokens_chain_id_enabled ON tokens(chain_id, enabled) WHERE enabled = true;
CREATE INDEX idx_tokens_symbol ON tokens(LOWER(symbol)) WHERE symbol IS NOT NULL;
CREATE INDEX idx_tokens_address ON tokens(LOWER(address));

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
-- Invoices table (network-agnostic)
-- An invoice is a payment request in a specific currency.
-- Payment options (which chains/assets can pay) are stored separately.
-- =============================================================================
CREATE TABLE invoices (
    id VARCHAR(64) PRIMARY KEY,  -- UUID string

    -- Link to store (required - invoices belong to stores)
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,

    -- Invoice currency (e.g., "USD", "EUR", "BTC", "ETH")
    -- This is what the merchant prices in
    currency VARCHAR(16) NOT NULL,

    -- Amount in the invoice currency
    -- For fiat, this is a decimal (e.g., 100.00)
    -- For crypto, this can be in smallest unit or human-readable
    amount NUMERIC(78, 18) NOT NULL,

    -- Total amount received across all payment options
    amount_received NUMERIC(78, 18) NOT NULL DEFAULT 0,

    -- Status
    status invoice_status NOT NULL DEFAULT 'pending',

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,

    -- Optional metadata (encrypted by client if sensitive)
    metadata JSONB,

    -- Optional extra data
    extra JSONB
);

-- Indexes for invoices
CREATE INDEX idx_invoices_status ON invoices(status);
CREATE INDEX idx_invoices_currency ON invoices(currency);
CREATE INDEX idx_invoices_store_id ON invoices(store_id);
CREATE INDEX idx_invoices_expires_at ON invoices(expires_at);
CREATE INDEX idx_invoices_created_at ON invoices(created_at);
CREATE INDEX idx_invoices_pending ON invoices(status, expires_at)
    WHERE status IN ('pending', 'processing', 'partially_paid');

-- =============================================================================
-- Payment options table
-- One payment option per payment method (e.g., ETH on Ethereum, USDC on Polygon)
-- =============================================================================
CREATE TABLE payment_options (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    invoice_id VARCHAR(64) NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,

    -- Payment method identifier (e.g., "ETH-1", "USDC-137", "ETH-11155111")
    payment_method_id VARCHAR(64) NOT NULL,

    -- Chain info
    chain_id BIGINT NOT NULL,

    -- Asset info
    asset_type asset_type NOT NULL,
    asset_symbol VARCHAR(32) NOT NULL,
    token_address VARCHAR(42),  -- For ERC20 tokens
    decimals SMALLINT NOT NULL DEFAULT 18,

    -- Payment destination
    payment_address VARCHAR(42) NOT NULL,

    -- Amount to pay in this asset (smallest unit)
    amount NUMERIC(78, 0) NOT NULL,

    -- Exchange rate info (if converted from fiat)
    rate NUMERIC(38, 18),              -- 1 invoice_currency = rate asset_units
    rate_at TIMESTAMPTZ,               -- When rate was fetched

    -- State
    is_active BOOLEAN NOT NULL DEFAULT TRUE,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique constraint: one payment method per invoice
    CONSTRAINT unique_payment_option UNIQUE (invoice_id, payment_method_id),

    -- Token constraint: ERC20 must have token_address
    CONSTRAINT payment_option_token_required CHECK (
        (asset_type = 'native' AND token_address IS NULL) OR
        (asset_type = 'erc20' AND token_address IS NOT NULL)
    )
);

-- Indexes for payment_options
CREATE INDEX idx_payment_options_invoice ON payment_options(invoice_id);
CREATE INDEX idx_payment_options_chain_id ON payment_options(chain_id);
CREATE INDEX idx_payment_options_address ON payment_options(payment_address, chain_id);
CREATE INDEX idx_payment_options_active ON payment_options(is_active) WHERE is_active = TRUE;

-- =============================================================================
-- Watched addresses table (for payment monitoring)
-- =============================================================================
CREATE TABLE watched_addresses (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    invoice_id VARCHAR(64) NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    payment_option_id UUID NOT NULL REFERENCES payment_options(id) ON DELETE CASCADE,

    -- Address info (denormalized for quick lookup)
    address VARCHAR(42) NOT NULL,          -- Checksummed EVM address
    chain_id BIGINT NOT NULL,
    token_address VARCHAR(42),             -- For ERC20 tokens

    -- Monitoring state
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    last_checked_block BIGINT,
    monitor_notified BOOLEAN NOT NULL DEFAULT FALSE,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,

    CONSTRAINT unique_watched_address UNIQUE (address, chain_id, token_address)
);

-- Indexes for watched_addresses
CREATE INDEX idx_watched_active ON watched_addresses(is_active, chain_id) WHERE is_active = TRUE;
CREATE INDEX idx_watched_invoice ON watched_addresses(invoice_id);
CREATE INDEX idx_watched_payment_option ON watched_addresses(payment_option_id);
CREATE INDEX idx_watched_address ON watched_addresses(address, chain_id);
CREATE INDEX idx_watched_expires ON watched_addresses(expires_at);
CREATE INDEX idx_watched_pending ON watched_addresses(monitor_notified, is_active)
    WHERE monitor_notified = FALSE AND is_active = TRUE;

-- =============================================================================
-- Payments table
-- =============================================================================
CREATE TABLE payments (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    invoice_id VARCHAR(64) NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    payment_option_id UUID REFERENCES payment_options(id) ON DELETE SET NULL,

    -- Chain info
    chain_id BIGINT NOT NULL,

    -- Asset info
    asset_type asset_type NOT NULL,
    asset_symbol VARCHAR(32) NOT NULL,
    token_address VARCHAR(42),             -- For ERC20 tokens

    -- Amount received (in smallest unit)
    amount NUMERIC(78, 0) NOT NULL,

    -- Transaction details
    tx_hash VARCHAR(66) NOT NULL,          -- 0x + 64 hex chars
    block_number BIGINT,                   -- Confirmations computed as: current_block - block_number + 1
    from_address VARCHAR(42),              -- Checksummed EVM address

    -- Timestamps
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at TIMESTAMPTZ,              -- NULL = awaiting confirmation, set when threshold reached

    -- Reorg handling
    reorged BOOLEAN NOT NULL DEFAULT FALSE,

    -- Chain-specific extra data
    extra JSONB,

    CONSTRAINT unique_payment_tx UNIQUE (tx_hash, chain_id)
);

-- Indexes for payments
CREATE INDEX idx_payments_invoice_id ON payments(invoice_id);
CREATE INDEX idx_payments_payment_option ON payments(payment_option_id);
CREATE INDEX idx_payments_tx_hash ON payments(tx_hash);
CREATE INDEX idx_payments_chain_id ON payments(chain_id);
CREATE INDEX idx_payments_detected_at ON payments(detected_at);
CREATE INDEX idx_payments_awaiting_confirmation ON payments(confirmed_at) WHERE confirmed_at IS NULL AND reorged = FALSE;

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
-- Chain configuration table
-- Stores confirmation requirements, explorer URLs, etc. per chain
-- =============================================================================
CREATE TABLE chain_configs (
    chain_id BIGINT PRIMARY KEY,           -- EIP-155 chain ID

    -- Chain metadata
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
-- Seed default chain configurations
-- =============================================================================
INSERT INTO chain_configs (chain_id, name, native_symbol, native_decimals, explorer_url, required_confirmations) VALUES
    -- Mainnets
    (1, 'Ethereum', 'ETH', 18, 'https://etherscan.io', 12),
    (137, 'Polygon', 'POL', 18, 'https://polygonscan.com', 128),
    (42161, 'Arbitrum', 'ETH', 18, 'https://arbiscan.io', 12),
    (10, 'Optimism', 'ETH', 18, 'https://optimistic.etherscan.io', 12),
    (8453, 'Base', 'ETH', 18, 'https://basescan.org', 12),
    (43114, 'Avalanche', 'AVAX', 18, 'https://snowtrace.io', 12),
    (56, 'BNB Chain', 'BNB', 18, 'https://bscscan.com', 15),
    (324, 'zkSync Era', 'ETH', 18, 'https://explorer.zksync.io', 1),
    (59144, 'Linea', 'ETH', 18, 'https://lineascan.build', 12),
    (534352, 'Scroll', 'ETH', 18, 'https://scrollscan.com', 12),
    -- Testnets
    (11155111, 'Sepolia', 'ETH', 18, 'https://sepolia.etherscan.io', 3),
    (17000, 'Holesky', 'ETH', 18, 'https://holesky.etherscan.io', 3),
    (80002, 'Polygon Amoy', 'POL', 18, 'https://amoy.polygonscan.com', 3);

-- =============================================================================
-- Seed common tokens on major EVM networks
-- =============================================================================
INSERT INTO tokens (token_type, chain_id, address, symbol, name, decimals) VALUES
    -- Ethereum (chain_id = 1)
    ('erc20', 1, '0xdAC17F958D2ee523a2206206994597C13D831ec7', 'USDT', 'Tether USD', 6),
    ('erc20', 1, '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', 'USDC', 'USD Coin', 6),
    ('erc20', 1, '0x6B175474E89094C44Da98b954EedeAC495271d0F', 'DAI', 'Dai Stablecoin', 18),
    -- Polygon (chain_id = 137)
    ('erc20', 137, '0xc2132D05D31c914a87C6611C10748AEb04B58e8F', 'USDT', 'Tether USD', 6),
    ('erc20', 137, '0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359', 'USDC', 'USD Coin', 6),
    -- Arbitrum (chain_id = 42161)
    ('erc20', 42161, '0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9', 'USDT', 'Tether USD', 6),
    ('erc20', 42161, '0xaf88d065e77c8cC2239327C5EDb3A432268e5831', 'USDC', 'USD Coin', 6),
    -- Optimism (chain_id = 10)
    ('erc20', 10, '0x94b008aA00579c1307B0EF2c499aD98a8ce58e58', 'USDT', 'Tether USD', 6),
    ('erc20', 10, '0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85', 'USDC', 'USD Coin', 6),
    -- Base (chain_id = 8453)
    ('erc20', 8453, '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913', 'USDC', 'USD Coin', 6),
    -- BNB Smart Chain (chain_id = 56)
    ('erc20', 56, '0x55d398326f99059fF775485246999027B3197955', 'USDT', 'Tether USD', 18),
    ('erc20', 56, '0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d', 'USDC', 'USD Coin', 18),
    -- Avalanche (chain_id = 43114)
    ('erc20', 43114, '0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7', 'USDT', 'Tether USD', 6),
    ('erc20', 43114, '0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E', 'USDC', 'USD Coin', 6);

-- =============================================================================
-- Helper functions
-- =============================================================================

-- Function to update invoice amount_received when payment is detected
-- Note: Status transitions to 'paid' are handled by the application after confirmations are verified
-- Reorged payments are excluded from amount_received calculation
CREATE OR REPLACE FUNCTION update_invoice_on_payment()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE invoices
    SET amount_received = (
        SELECT COALESCE(SUM(amount), 0)
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

-- =============================================================================
-- Comments for documentation
-- =============================================================================
COMMENT ON TABLE tokens IS 'Registry of known ERC20 tokens across EVM networks';
COMMENT ON TABLE invoices IS 'Network-agnostic payment invoices';
COMMENT ON TABLE payment_options IS 'Payment options for invoices - one per chain/asset combination';
COMMENT ON TABLE payments IS 'Received payments linked to invoices';
COMMENT ON TABLE watched_addresses IS 'Addresses being monitored for incoming payments';
COMMENT ON TABLE payment_events IS 'Audit log of payment-related events';
COMMENT ON TABLE chain_configs IS 'Configuration for supported EVM chains';

COMMENT ON COLUMN invoices.currency IS 'Invoice currency (e.g., USD, EUR, BTC, ETH)';
COMMENT ON COLUMN invoices.amount IS 'Invoice amount in the currency';
COMMENT ON COLUMN payment_options.payment_method_id IS 'Format: {ASSET}-{CHAIN_ID} e.g., ETH-1, USDC-137';
COMMENT ON COLUMN payment_options.amount IS 'Amount in smallest unit (wei for ETH, token decimals for ERC20)';
COMMENT ON COLUMN payment_options.rate IS 'Exchange rate: 1 invoice_currency = rate asset_units';
COMMENT ON COLUMN payments.tx_hash IS 'Transaction hash (0x + 64 hex chars)';
COMMENT ON COLUMN watched_addresses.payment_option_id IS 'The payment option this address is watching for';
