CREATE OR REPLACE VIEW expected_watched_addresses AS
SELECT
    wa.address,
    wa.chain_id,
    wa.token_address,
    wa.payment_option_id,
    po.invoice_id
FROM watched_addresses wa
JOIN payment_options po ON wa.payment_option_id = po.id
JOIN invoices i ON po.invoice_id = i.id
WHERE wa.is_active = TRUE
  AND (
    i.status IN ('pending', 'processing', 'partially_paid', 'cancelled')
    OR (
      i.status = 'expired'
      AND i.expires_at > NOW() - INTERVAL '1 day' -- GRACE_CEILING
    )
    OR (
      i.status = 'paid'
      AND EXISTS (
        SELECT 1 FROM payments p
        WHERE p.invoice_id = i.id
          AND p.confirmed_at > NOW() - INTERVAL '1 day' -- GRACE_CEILING
      )
    )
  );
