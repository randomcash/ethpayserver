-- WARNING: this discards the only record of what each account's recovery key
-- was actually salted with.
--
-- A down-then-up cycle re-runs the backfill against email/wallet fields that may
-- have changed in the meantime, silently pinning a different identifier than the
-- one recovery_verification_hash was built from — reintroducing exactly the
-- permanent unrecoverability RCS-201 exists to prevent, with no error at any
-- point. Dump the column before rolling back if any account has been created
-- since it was applied.
ALTER TABLE users DROP COLUMN IF EXISTS kdf_salt_identifier;
