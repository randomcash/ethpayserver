-- RCS-201, 2 of 2: backfill. Separate file so the ALTER's lock is already
-- released; this runs at ROW EXCLUSIVE and does not block reads.
--
-- Precedence is wallet-before-email, the REVERSE of User::kdf_salt_identifier().
--
-- That is deliberate. Registration has no email path — `User::new` has zero call
-- sites, and the client only ever produces `wallet:{addr}` or `passkey:{id}`. So
-- a row holding BOTH an email and a wallet is, by construction, an account that
-- registered with a wallet and acquired an email afterwards: precisely the
-- RCS-201 victim. Pinning `email` would freeze the broken derivation forever;
-- pinning the wallet restores the value their stored hash was built from.
--
-- Email is lowercased to match `User::new`, which pins `email.to_lowercase()`.
-- Pinning a mixed-case value would freeze a salt no client reproduces.
UPDATE users
SET kdf_salt_identifier = CASE
    WHEN primary_wallet_address IS NOT NULL THEN 'wallet:' || primary_wallet_address
    WHEN email IS NOT NULL                  THEN LOWER(email)
    ELSE 'passkey:' || id::text
END
WHERE kdf_salt_identifier IS NULL;
