-- no-transaction
-- RCS-201: pin the recovery KDF salt identifier at registration.
--
-- Runs outside a transaction (see the directive on line 1) so the ALTER commits
-- and releases its ACCESS EXCLUSIVE lock before the backfill starts. In one
-- transaction the lock would be held across the whole UPDATE, blocking every
-- login and session read on `users` while the old server is still serving.
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

-- Nullable, and deliberately NOT tightened to NOT NULL here. The migrate
-- container runs while the *previous* server is still serving: any registration
-- committing between the backfill and a `SET NOT NULL` would insert a NULL row
-- (the old binary's INSERT has no such column) and abort the whole migration,
-- leaving the new server unable to start. The reader falls back to the computed
-- value when this is NULL, so the rolling window is safe; a follow-up migration
-- can tighten it once the new binary is the only writer.
ALTER TABLE users
    ADD COLUMN kdf_salt_identifier VARCHAR(255);

-- Backfill. Note the precedence is wallet-before-email, which is the REVERSE of
-- User::kdf_salt_identifier().
--
-- That is deliberate. Registration has no email path — `User::new` has zero call
-- sites, and the client only ever produces `wallet:{addr}` or `passkey:{id}`. So
-- a row holding BOTH an email and a wallet is, by construction, an account that
-- registered with a wallet and acquired an email afterwards: precisely the
-- RCS-201 victim whose recovery is currently broken.
--
-- Pinning `email` for those rows would freeze the broken derivation forever.
-- Pinning the wallet restores the value their recovery_verification_hash was
-- actually built from, and makes them recoverable again.
UPDATE users
SET kdf_salt_identifier = CASE
    WHEN primary_wallet_address IS NOT NULL THEN 'wallet:' || primary_wallet_address
    WHEN email IS NOT NULL                  THEN email
    ELSE 'passkey:' || id::text
END
WHERE kdf_salt_identifier IS NULL;

COMMENT ON COLUMN users.kdf_salt_identifier IS
    'Identifier the recovery KDF was salted with at registration. Immutable: '
    'changing it invalidates recovery_verification_hash and makes the account '
    'unrecoverable. NULL means pre-RCS-201; readers fall back to the computed '
    'value (RCS-201).';
