-- Refund and payout tables for RCS-74.
-- Refunds: send funds back to original payer from derived payment addresses.
-- Payouts: sweep confirmed funds from derived addresses to merchant wallet.

CREATE TABLE refunds (
    id UUID PRIMARY KEY,
    invoice_id TEXT NOT NULL,
    payment_id UUID NOT NULL REFERENCES payments(id),
    store_id UUID NOT NULL REFERENCES stores(id),
    to_address TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    asset_type TEXT NOT NULL DEFAULT 'native',
    asset_symbol TEXT NOT NULL,
    token_address TEXT,
    amount TEXT NOT NULL,
    tx_hash TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    fee_amount TEXT,
    reason TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at TIMESTAMPTZ
);

CREATE INDEX idx_refunds_invoice_id ON refunds(invoice_id);
CREATE INDEX idx_refunds_store_id ON refunds(store_id, created_at DESC);
CREATE INDEX idx_refunds_status ON refunds(status) WHERE status IN ('pending', 'broadcasting');

CREATE TABLE payouts (
    id UUID PRIMARY KEY,
    store_id UUID NOT NULL REFERENCES stores(id),
    invoice_ids JSONB NOT NULL DEFAULT '[]',
    destination_address TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    asset_type TEXT NOT NULL DEFAULT 'native',
    asset_symbol TEXT NOT NULL,
    token_address TEXT,
    amount TEXT NOT NULL,
    tx_hash TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    fee_amount TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at TIMESTAMPTZ
);

CREATE INDEX idx_payouts_store_id ON payouts(store_id, created_at DESC);
CREATE INDEX idx_payouts_status ON payouts(status) WHERE status IN ('pending', 'broadcasting');
