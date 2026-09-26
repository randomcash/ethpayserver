-- The authoritative definition of "should currently be watched".
--
-- `watched_addresses` cascades away with the invoice it belongs to, so a
-- Postgres-only check for an "orphaned" row can never find one - the foreign
-- keys already guarantee it can't exist. The state that actually goes stale
-- lives in Redis (the monitor's own watch set), which nothing here can see.
-- What this view gives the comparison in the application layer is the other
-- half: the set Redis's watch set should equal, so a mismatch in either
-- direction becomes visible.
--
-- `is_active = TRUE` is necessary but not sufficient: the background cleanup
-- jobs (`get_paid_for_cleanup`, `get_cancelled_for_cleanup`,
-- `get_expired_for_cleanup`) only flip it to `FALSE` once they run, so a
-- watched address can sit `is_active = TRUE` for a resolved invoice in the
-- window before cleanup executes - or forever, if the unwatch step that
-- follows it never fires. Scoping by invoice status here, the same statuses
-- `idx_invoices_pending` already treats as unresolved, is what excludes that
-- window rather than reporting it as still expected.
CREATE VIEW expected_watched_addresses AS
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
  AND wa.expires_at > NOW()
  AND i.status IN ('pending', 'processing', 'partially_paid');
