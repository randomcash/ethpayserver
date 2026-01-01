-- Payment aggregation support
-- Adds columns to store converted amounts in invoice currency for proper aggregation

-- Add converted amount columns to payments table
ALTER TABLE payments
    ADD COLUMN credited_amount NUMERIC(78, 18),
    ADD COLUMN rate_used NUMERIC(38, 18),
    ADD COLUMN rate_applied_at TIMESTAMPTZ;

-- The credited_amount column stores the payment's value in invoice currency terms.
-- This enables aggregating payments across different assets (ETH, USDC, etc.) into
-- a single total that can be compared against the invoice amount.
--
-- For cross-currency payments (e.g., USD invoice paid with ETH):
--   credited_amount = (raw_amount / 10^decimals) / rate
--
-- For same-asset payments (e.g., ETH invoice paid with ETH):
--   credited_amount = raw_amount / 10^decimals
--
-- Payments with NULL credited_amount are recorded but do NOT count toward amount_received.
COMMENT ON COLUMN payments.credited_amount IS 'Payment value in invoice currency (for aggregation). NULL means payment is not counted toward invoice total.';
COMMENT ON COLUMN payments.rate_used IS 'Exchange rate used for conversion (locked at invoice creation). Format: 1 invoice_currency = rate asset_units.';
COMMENT ON COLUMN payments.rate_applied_at IS 'Timestamp when rate was applied to calculate credited_amount.';

-- Update the trigger to sum credited amounts
-- IMPORTANT: Only payments with credited_amount are counted.
-- Payments without conversion (missing payment option, conversion failure) are excluded.
-- This is intentional - we can't safely aggregate raw amounts (in wei) with invoice currency amounts.
CREATE OR REPLACE FUNCTION update_invoice_on_payment()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE invoices
    SET amount_received = (
        SELECT COALESCE(SUM(credited_amount), 0)
        FROM payments
        WHERE invoice_id = NEW.invoice_id
          AND reorged = FALSE
          AND credited_amount IS NOT NULL
    ),
    status = CASE
        WHEN status = 'pending' AND NEW.reorged = FALSE THEN 'processing'::invoice_status
        ELSE status
    END
    WHERE id = NEW.invoice_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Index for efficient payment lookups by payment option
CREATE INDEX IF NOT EXISTS idx_payments_payment_option_id ON payments(payment_option_id)
    WHERE payment_option_id IS NOT NULL;
