-- WARNING: discards the only record of what each account's recovery key was
-- salted with. A down-then-up cycle re-derives it from email/wallet fields that
-- may have changed since, silently pinning a value the stored
-- recovery_verification_hash was not built from — reintroducing exactly the
-- permanent unrecoverability RCS-201 exists to prevent. Dump the column first.
ALTER TABLE users DROP COLUMN IF EXISTS kdf_salt_identifier;
