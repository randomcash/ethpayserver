-- Server-wide settings table (single-row pattern).
-- Stores admin-configurable payment defaults, rate limits, and enabled chains.
CREATE TABLE IF NOT EXISTS server_settings (
    id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    default_confirmations INTEGER NOT NULL DEFAULT 3,
    invoice_expiry_minutes INTEGER NOT NULL DEFAULT 60,
    rate_limit_rpm INTEGER NOT NULL DEFAULT 100,
    enabled_chain_ids BIGINT[] NOT NULL DEFAULT ARRAY[1, 10, 137, 42161, 8453, 56, 43114, 250, 100, 324, 59144, 534352],
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
