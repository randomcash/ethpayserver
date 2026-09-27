-- Restore the comment as 20260923000001 left it.
COMMENT ON COLUMN server_settings.operator_store_id IS
    'The operator''s own store: where this instance issues its own subscription '
    'invoices. Read at boot; a change takes effect on restart. NULL means the '
    'instance issues no invoices to itself.';
