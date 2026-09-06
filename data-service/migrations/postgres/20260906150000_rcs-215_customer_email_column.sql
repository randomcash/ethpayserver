-- RCS-215: make customer_email a real column instead of one generated from metadata.
--
-- It was previously derived by Postgres:
--
--     ADD COLUMN customer_email VARCHAR(320) GENERATED ALWAYS AS (
--         COALESCE(metadata->>'customer_email', metadata->>'buyer_email')
--     ) STORED;
--
-- That cannot survive metadata becoming ciphertext (RCS-216). It would not
-- error - it would silently return NULL, and customer receipts would stop
-- being sent with nothing in the logs explaining why.
--
-- Order matters: the value is copied out of the generated column BEFORE it is
-- dropped, so existing rows keep their address. Doing this after metadata is
-- ever encrypted would strand that data permanently.

ALTER TABLE invoices ADD COLUMN customer_email_new VARCHAR(320);

UPDATE invoices SET customer_email_new = customer_email WHERE customer_email IS NOT NULL;

DROP INDEX IF EXISTS idx_invoices_customer_email;
ALTER TABLE invoices DROP COLUMN customer_email;
ALTER TABLE invoices RENAME COLUMN customer_email_new TO customer_email;

-- The old index was write-only: no query filtered, searched or sorted by this
-- column. It is deliberately not recreated. Add one when a query needs it.
