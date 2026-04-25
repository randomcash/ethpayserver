-- Wallet rotation history for xpub key rotation (RCS-117)
-- Tracks each rotation event so archived xpubs are preserved for audit.

CREATE TABLE wallet_rotations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    store_id UUID NOT NULL REFERENCES stores(id) ON DELETE CASCADE,
    previous_xpub VARCHAR(120) NOT NULL,
    new_xpub VARCHAR(120) NOT NULL,
    payment_method_id UUID NOT NULL REFERENCES store_payment_methods(id) ON DELETE CASCADE,
    previous_derivation_index INTEGER NOT NULL DEFAULT 0,
    reason TEXT,
    rotated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_wallet_rotations_store ON wallet_rotations(store_id);
CREATE INDEX idx_wallet_rotations_payment_method ON wallet_rotations(payment_method_id);

COMMENT ON TABLE wallet_rotations IS 'Audit log of xpub rotations per payment method';
COMMENT ON COLUMN wallet_rotations.previous_xpub IS 'The xpub that was replaced';
COMMENT ON COLUMN wallet_rotations.new_xpub IS 'The xpub that replaced it';
COMMENT ON COLUMN wallet_rotations.payment_method_id IS 'The payment method whose xpub was rotated';
COMMENT ON COLUMN wallet_rotations.previous_derivation_index IS 'Derivation index at time of rotation (addresses below this are still watched)';
