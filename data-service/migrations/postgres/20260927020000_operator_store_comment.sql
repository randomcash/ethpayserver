-- Re-issue the comment on server_settings.operator_store_id.
--
-- The column was renamed in 20260923000001, and that migration's COMMENT still
-- describes what the instance invoices itself for in commercial terms. These
-- repositories are public and the comment ships with the schema: pg_dump emits
-- it, \d+ prints it, and any reader of the database sees it.
--
-- It cannot be fixed where it was written. sqlx stores a SHA-384 of each
-- migration file and compares it on startup, so editing the bytes of a
-- migration that has already been applied breaks every deploy from then on,
-- not only the first. Renaming a migration file is free; changing its contents
-- is not. So the correction is a new migration that overwrites the comment,
-- which is idempotent for a comment and costs one statement.

COMMENT ON COLUMN server_settings.operator_store_id IS
    'The operator''s own store: where this instance issues its own invoices. '
    'Read at boot; a change takes effect on restart. NULL means the instance '
    'issues no invoices to itself.';
