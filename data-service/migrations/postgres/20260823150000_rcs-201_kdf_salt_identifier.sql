-- RCS-201: pin the recovery KDF salt identifier at registration.
--
-- `recovery_verification_hash` is written once, from
--   Argon2id(BIP39-seed(phrase), "payserver-recovery:{identifier}")
-- but the identifier was *recomputed* on every recovery attempt by
-- User::kdf_salt_identifier(), which prefers email, then wallet, then passkey.
--
-- So a user who registered with a wallet and later added an email silently
-- changed what the server derives, while the stored hash never moved. Recovery
-- then fails permanently, with a phrase the user recorded correctly, and the
-- failure only surfaces at the moment they actually need it.
--
-- Storing the identifier makes it immutable by construction.

ALTER TABLE users
    ADD COLUMN kdf_salt_identifier VARCHAR(255);

-- Backfill with exactly what kdf_salt_identifier() would return today. This is
-- correct for every existing account: none can have changed identity yet
-- without already being broken, and this pins whatever they registered under.
UPDATE users
SET kdf_salt_identifier = CASE
    WHEN email IS NOT NULL                  THEN email
    WHEN primary_wallet_address IS NOT NULL  THEN 'wallet:' || primary_wallet_address
    ELSE 'passkey:' || id::text
END
WHERE kdf_salt_identifier IS NULL;

ALTER TABLE users
    ALTER COLUMN kdf_salt_identifier SET NOT NULL;

COMMENT ON COLUMN users.kdf_salt_identifier IS
    'Identifier the recovery KDF was salted with at registration. Immutable: '
    'changing it invalidates recovery_verification_hash and makes the account '
    'unrecoverable (RCS-201).';
