-- Restore the generated column. Any address written directly (rather than via
-- metadata) is lost on the way back - the generated column can only derive.
ALTER TABLE invoices DROP COLUMN IF EXISTS customer_email;

ALTER TABLE invoices
ADD COLUMN customer_email VARCHAR(320) GENERATED ALWAYS AS (
    COALESCE(metadata->>'customer_email', metadata->>'buyer_email')
) STORED;

CREATE INDEX idx_invoices_customer_email ON invoices (customer_email)
    WHERE customer_email IS NOT NULL;
