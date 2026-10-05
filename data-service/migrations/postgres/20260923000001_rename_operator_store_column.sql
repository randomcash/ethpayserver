-- The prior migration's column was named after the commercial model this
-- setting happens to serve. The capability is neutral - the server just
-- needs to know which store it issues its own invoices through - so the
-- column is renamed to describe what it is rather than what it is for.
--
-- A rename, not a new column: the values already stored are still correct
-- for the store they name, and a fresh column would need its own backfill
-- and a window where both existed.
ALTER TABLE server_settings
    RENAME COLUMN billing_store_id TO operator_store_id;

COMMENT ON COLUMN server_settings.operator_store_id IS
    'The operator''s own store: where this instance issues its own subscription '
    'invoices. Read at boot; a change takes effect on restart. NULL means the '
    'instance issues no invoices to itself.';
