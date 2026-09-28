-- `expected_watched_addresses` (see that migration for the full reasoning)
-- omitted `late_paid`. No cleanup job ever selects it:
-- `InvoiceCleanupService::cleanup_addresses` only runs
-- `cleanup_expired_addresses`/`cleanup_paid_addresses`/
-- `cleanup_cancelled_addresses`, filtered to `'expired'`/`'paid'`/
-- `'cancelled'` respectively - none match `'late_paid'`. So a late-paid
-- invoice's `watched_addresses` row never has `is_active` flipped to
-- `FALSE`, and the monitor is never told to unwatch it: Redis and
-- `is_active` stay in agreement, correctly still watching. Leaving
-- `late_paid` out of "expected" reported that correct, permanent agreement
-- as a stale watch forever, since unlike `paid`/`expired`/`cancelled` there
-- is no later cleanup pass for the mismatch to resolve against.
--
-- Included unconditionally, the same way as `cancelled`: there is no
-- timestamp on `invoices` marking when a late payment landed to bound a
-- grace window against, and no cleanup pass will ever move it out of this
-- set, so a ceiling would only reintroduce the permanent-false-positive
-- this migration exists to fix.
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
    i.status IN ('pending', 'processing', 'partially_paid', 'cancelled', 'late_paid')
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
