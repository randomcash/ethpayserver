-- Store wallet configuration for payment address derivation
-- Merchants provide their xpub (extended public key) to receive payments
-- Server derives unique addresses for each invoice without holding private keys

CREATE TABLE store_wallets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,

    -- Wallet configuration (xpub for now, extensible for future methods)
    xpub VARCHAR(120) NOT NULL,

    -- Derivation tracking
    derivation_index INTEGER NOT NULL DEFAULT 0,

    -- Metadata
    name VARCHAR(100),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One wallet per store for now
    UNIQUE(store_id)
);

CREATE INDEX idx_store_wallets_store_id ON store_wallets(store_id);

-- Add derivation_index to invoices to track which index was used
ALTER TABLE invoices
    ADD COLUMN derivation_index INTEGER;

COMMENT ON TABLE store_wallets IS 'Wallet configuration for stores to receive payments';
COMMENT ON COLUMN store_wallets.xpub IS 'BIP-32 extended public key for deriving payment addresses';
COMMENT ON COLUMN store_wallets.derivation_index IS 'Next derivation index to use for invoice addresses';
COMMENT ON COLUMN invoices.derivation_index IS 'Derivation index used for this invoice payment address';
