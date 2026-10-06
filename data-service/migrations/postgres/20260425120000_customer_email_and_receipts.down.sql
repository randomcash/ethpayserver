DROP INDEX IF EXISTS idx_invoices_customer_email;
ALTER TABLE invoices DROP COLUMN IF EXISTS customer_email;
ALTER TABLE store_settings DROP COLUMN IF EXISTS customer_receipts_enabled;
