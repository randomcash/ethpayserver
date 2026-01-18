-- Restore the original constraint (will fail if passkey-only users exist)
ALTER TABLE users ADD CONSTRAINT users_has_identifier CHECK (
    email IS NOT NULL OR primary_wallet_address IS NOT NULL
);
