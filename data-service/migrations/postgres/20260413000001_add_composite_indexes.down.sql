-- Reverse composite indexes migration

-- Restore dropped single-column indexes
CREATE INDEX idx_payments_invoice_id ON payments(invoice_id);
CREATE INDEX idx_payment_options_address ON payment_options(payment_address, chain_id);

-- Drop new composite indexes
DROP INDEX IF EXISTS idx_invoices_store_status;
DROP INDEX IF EXISTS idx_invoices_store_created;
DROP INDEX IF EXISTS idx_payments_invoice_valid;
DROP INDEX IF EXISTS idx_payments_invoice_detected;
DROP INDEX IF EXISTS idx_payments_reorg_check;
DROP INDEX IF EXISTS idx_watched_lower_addr_chain;
DROP INDEX IF EXISTS idx_watched_active_expiry;
DROP INDEX IF EXISTS idx_payment_options_invoice_active;
DROP INDEX IF EXISTS idx_payment_options_addr_chain_token;
