-- Fix invoice expiration lifecycle gaps
--
-- 1. Guard the payment trigger against transitioning expired invoices
-- 2. Align expire_old_invoices() to include processing and partially_paid

-- Fix: update_invoice_on_payment should NOT transition expired invoices
-- back to 'processing'. Only transition if invoice is 'pending' AND not past
-- its expiration time.
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
        WHEN status = 'pending' AND NEW.reorged = FALSE AND expires_at >= NOW()
            THEN 'processing'::invoice_status
        ELSE status
    END
    WHERE id = NEW.invoice_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Align: expire_old_invoices() should also expire processing and partially_paid
-- invoices that have timed out, not just pending ones.
CREATE OR REPLACE FUNCTION expire_old_invoices()
RETURNS INTEGER AS $$
DECLARE
    expired_count INTEGER;
BEGIN
    UPDATE invoices
    SET status = 'expired'
    WHERE status IN ('pending', 'processing', 'partially_paid')
    AND expires_at < NOW();

    GET DIAGNOSTICS expired_count = ROW_COUNT;
    RETURN expired_count;
END;
$$ LANGUAGE plpgsql;
