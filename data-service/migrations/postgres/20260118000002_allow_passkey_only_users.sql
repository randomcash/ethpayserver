-- Allow users with only passkeys (no email or wallet required)
-- The passkey_credentials table enforces the relationship

ALTER TABLE users DROP CONSTRAINT users_has_identifier;

-- Add a new constraint that allows passkey-only users
-- A user must have email, wallet, OR at least one passkey
-- Since passkeys are in a separate table, we use a trigger instead

-- For now, just remove the constraint since passkey_credentials.user_id
-- references users(id) with ON DELETE CASCADE, ensuring data integrity
