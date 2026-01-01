-- Revert payment aggregation changes

-- Drop index
DROP INDEX IF EXISTS idx_payments_payment_option_id;

-- Restore original trigger (sums raw amounts)
CREATE OR REPLACE FUNCTION update_invoice_on_payment()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE invoices
    SET amount_received = (
        SELECT COALESCE(SUM(CAST(amount AS NUMERIC(78, 0))), 0)
        FROM payments
        WHERE invoice_id = NEW.invoice_id AND reorged = FALSE
    ),
    status = CASE
        WHEN status = 'pending' AND NEW.reorged = FALSE THEN 'processing'::invoice_status
        ELSE status
    END
    WHERE id = NEW.invoice_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Remove columns
ALTER TABLE payments
    DROP COLUMN IF EXISTS amount_in_invoice_currency,
    DROP COLUMN IF EXISTS rate_used,
    DROP COLUMN IF EXISTS rate_applied_at;
