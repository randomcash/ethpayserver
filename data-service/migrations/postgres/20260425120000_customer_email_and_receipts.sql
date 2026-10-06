-- Add customer_email column to invoices (generated from metadata for backward compat)
-- and customer_receipts_enabled toggle to store_settings.

-- Dedicated column derived from the metadata JSONB field.
-- Existing invoices that already carry metadata->>'customer_email' or
-- metadata->>'buyer_email' are populated automatically.
ALTER TABLE invoices
ADD COLUMN customer_email VARCHAR(320) GENERATED ALWAYS AS (
    COALESCE(metadata->>'customer_email', metadata->>'buyer_email')
) STORED;

CREATE INDEX idx_invoices_customer_email ON invoices (customer_email)
    WHERE customer_email IS NOT NULL;

-- Per-store toggle (default true — receipts are sent when SMTP is configured).
ALTER TABLE store_settings
ADD COLUMN customer_receipts_enabled BOOLEAN NOT NULL DEFAULT TRUE;
