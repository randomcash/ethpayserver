-- Store payment methods - replaces store_wallets with multi-chain support
-- Each payment method = (chain, asset, wallet) configuration

CREATE TABLE store_payment_methods (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,

    -- Network identification (chain_id instead of network enum for flexibility)
    chain_id BIGINT NOT NULL,

    -- Asset: NULL token_address = native asset, otherwise ERC20
    token_address VARCHAR(42),
    asset_symbol VARCHAR(32) NOT NULL,
    decimals SMALLINT NOT NULL DEFAULT 18,  -- Token decimals (18 for ETH, 6 for USDC/USDT)

    -- Wallet for this network (xpub for HD derivation)
    xpub VARCHAR(120) NOT NULL,
    derivation_index INTEGER NOT NULL DEFAULT 0,

    -- Status
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One payment method per (store, chain, asset) combination
    UNIQUE(store_id, chain_id, token_address)
);

CREATE INDEX idx_store_payment_methods_store ON store_payment_methods(store_id);
CREATE INDEX idx_store_payment_methods_enabled ON store_payment_methods(store_id, enabled)
    WHERE enabled = true;

-- Add chain_id to invoices (replaces network enum for new invoices)
-- Note: currency column already exists in base table
ALTER TABLE invoices
    ADD COLUMN IF NOT EXISTS chain_id BIGINT;

-- Add chain_id to watched_addresses
ALTER TABLE watched_addresses
    ADD COLUMN IF NOT EXISTS chain_id BIGINT;

-- Note: Data migration from network enum to chain_id is not needed for fresh installs.
-- Legacy data migration removed (network_configs table doesn't exist).

COMMENT ON TABLE store_payment_methods IS 'Payment method configurations per store (chain + asset + wallet)';
COMMENT ON COLUMN store_payment_methods.chain_id IS 'EIP-155 chain ID (1=Ethereum, 137=Polygon, 11155111=Sepolia, etc.)';
COMMENT ON COLUMN store_payment_methods.token_address IS 'ERC20 token contract address, NULL for native asset';
COMMENT ON COLUMN store_payment_methods.asset_symbol IS 'Asset symbol for display (ETH, USDC, etc.)';
COMMENT ON COLUMN invoices.chain_id IS 'Chain ID for this invoice (replaces network enum)';
COMMENT ON COLUMN invoices.currency IS 'Currency the invoice amount is denominated in';

-- Drop deprecated store_wallets table (replaced by store_payment_methods)
DROP TABLE IF EXISTS store_wallets;

-- Drop derivation_index from invoices (now tracked in payment_options)
ALTER TABLE invoices DROP COLUMN IF EXISTS derivation_index;
