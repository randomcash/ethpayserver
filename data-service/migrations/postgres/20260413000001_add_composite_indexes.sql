-- Add composite database indexes for verified query patterns
-- Targets: invoices, payments, watched_addresses, payment_options
-- See: data-service/src/postgres/{invoice,payment,watched_address,payment_option}.rs

-- ============================================================================
-- Invoices: store-level composite indexes
-- ============================================================================

-- InvoiceReader::query() — filters by store_id + status (dashboard counts, invoice lists)
CREATE INDEX idx_invoices_store_status ON invoices(store_id, status);

-- InvoiceReader::query() — filters by store_id + ORDER BY created_at DESC (invoice listing)
CREATE INDEX idx_invoices_store_created ON invoices(store_id, created_at DESC);

-- ============================================================================
-- Payments: invoice-level composite indexes
-- ============================================================================

-- PaymentReader::get_valid_for_invoice(), has_valid_payments(), trigger SUM
-- Partial index covering the hot path (non-reorged payments)
CREATE INDEX idx_payments_invoice_valid ON payments(invoice_id) WHERE reorged = FALSE;

-- PaymentReader::get_for_invoice() — ORDER BY detected_at DESC
-- Also covers bare invoice_id lookups (replaces idx_payments_invoice_id)
CREATE INDEX idx_payments_invoice_detected ON payments(invoice_id, detected_at DESC);

-- PaymentWriter::mark_reorged() — UPDATE WHERE invoice_id + chain_id + block_number >= N
CREATE INDEX idx_payments_reorg_check ON payments(invoice_id, chain_id, block_number)
    WHERE reorged = FALSE;

-- ============================================================================
-- Watched Addresses: expression index for LOWER() lookups
-- ============================================================================

-- WatchedAddressReader::get_invoice_id() — WHERE LOWER(address) = LOWER($1) AND chain_id = $2
-- Existing idx_watched_address uses raw address; queries use LOWER()
CREATE INDEX idx_watched_lower_addr_chain ON watched_addresses(LOWER(address), chain_id)
    WHERE is_active = TRUE;

-- WatchedAddressReader::get_active() — WHERE is_active = TRUE AND expires_at > NOW()
CREATE INDEX idx_watched_active_expiry ON watched_addresses(expires_at)
    WHERE is_active = TRUE;

-- ============================================================================
-- Payment Options: active invoice lookups and address resolution
-- ============================================================================

-- PaymentOptionReader::get_active_for_invoice() — WHERE invoice_id = $1 AND is_active = TRUE
CREATE INDEX idx_payment_options_invoice_active ON payment_options(invoice_id)
    WHERE is_active = TRUE;

-- PaymentOptionReader::get_by_address() — WHERE payment_address + chain_id + token_address
-- Extends existing idx_payment_options_address with token_address column
CREATE INDEX idx_payment_options_addr_chain_token
    ON payment_options(payment_address, chain_id, token_address);

-- ============================================================================
-- Drop redundant single-column indexes (now strict left-prefixes of new composites)
-- ============================================================================

-- Covered by idx_payments_invoice_detected (same leading column, not partial)
DROP INDEX IF EXISTS idx_payments_invoice_id;

-- Covered by idx_payment_options_addr_chain_token (extends with token_address)
DROP INDEX IF EXISTS idx_payment_options_address;
