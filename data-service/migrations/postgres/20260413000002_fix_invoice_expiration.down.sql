-- Revert to previous trigger and function versions

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

CREATE OR REPLACE FUNCTION expire_old_invoices()
RETURNS INTEGER AS $$
DECLARE
    expired_count INTEGER;
BEGIN
    UPDATE invoices
    SET status = 'expired'
    WHERE status IN ('pending', 'processing')
    AND expires_at < NOW();

    GET DIAGNOSTICS expired_count = ROW_COUNT;
    RETURN expired_count;
END;
$$ LANGUAGE plpgsql;
