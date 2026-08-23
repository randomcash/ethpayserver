-- RCS-201, 1 of 2: add the column only.
--
-- Deliberately alone in this file. `ADD COLUMN` takes ACCESS EXCLUSIVE on
-- `users`; Postgres wraps a multi-statement simple Query (which is how sqlx
-- executes a migration file) in one implicit transaction, so pairing it with the
-- backfill would hold that lock across a full-table rewrite and block every
-- login and session read while the previous server is still serving. As its own
-- statement it commits immediately and the lock is gone.
--
-- IF NOT EXISTS because sqlx records the _sqlx_migrations row in a separate
-- statement: a crash between this succeeding and that INSERT would otherwise
-- make every retry die with "column already exists" and crash-loop the migrate
-- container.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS kdf_salt_identifier VARCHAR(255);

COMMENT ON COLUMN users.kdf_salt_identifier IS
    'Identifier the recovery KDF was salted with at registration. Immutable: '
    'changing it invalidates recovery_verification_hash and makes the account '
    'unrecoverable. NULL means pre-RCS-201; readers fall back to the computed '
    'value (RCS-201).';
