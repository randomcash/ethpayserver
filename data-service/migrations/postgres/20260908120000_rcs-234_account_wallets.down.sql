-- RCS-234, down. Reversible only while nothing has been shared yet.
--
-- The up migration consolidated N payment-method counters onto one wallet
-- counter. Going back means splitting one counter into N, and there is no
-- correct split. Each method's old value is gone, and every candidate is
-- wrong in a way that costs money:
--
--   * copy the shared value to each method - each then counts on from the same
--     number, and the next address every one of them issues is identical. That
--     is the exact bug this ticket removed, reintroduced by the rollback.
--   * reset them to 0 - re-issues every address the wallet ever handed out,
--     including ones already holding funds.
--   * distribute the range - invents an assignment that never happened, and
--     any error re-issues an address anyway.
--
-- So: reverse it where the reversal is exact, and refuse where it is not.
-- A wallet backing a single payment method was never consolidated with
-- anything, and copying its xpub and counter back is lossless - that covers
-- rolling back a bad deploy, which is what a down migration is actually for.
-- A wallet backing two or more is a merge that cannot be unmerged, and the
-- migration stops rather than corrupt derivation quietly.
--
-- If you hit the exception and truly must go back, the honest procedure is to
-- restore from a backup taken before the up migration. Any in-place split is a
-- guess.

DO $$
DECLARE shared INTEGER;
BEGIN
    SELECT COUNT(*) INTO shared FROM (
        SELECT wallet_id FROM store_payment_methods
        GROUP BY wallet_id HAVING COUNT(*) > 1
    ) AS merged;

    IF shared > 0 THEN
        RAISE EXCEPTION
            'RCS-234 down: % wallet(s) are shared by more than one payment '
            'method. Their per-method derivation indices were merged into one '
            'counter and cannot be split back without re-deriving addresses '
            'that have already been issued. Restore from a pre-migration '
            'backup instead.', shared;
    END IF;
END $$;

-- Losses on the way back, stated rather than hidden:
--   * per-store overrides (store_wallets) - the old shape had no such concept.
--   * wallet names, is_primary, and any wallet not referenced by a payment
--     method - likewise no home in the old shape.
--   * payment_options provenance (wallet_id, derivation_index).
-- Addresses already issued are unaffected: payment_address is untouched.

ALTER TABLE payment_options DROP COLUMN IF EXISTS derivation_index;
ALTER TABLE payment_options DROP COLUMN IF EXISTS wallet_id;

ALTER TABLE store_payment_methods ADD COLUMN xpub VARCHAR(120);
ALTER TABLE store_payment_methods ADD COLUMN derivation_index INTEGER NOT NULL DEFAULT 0;

UPDATE store_payment_methods pm
SET xpub = w.xpub, derivation_index = w.derivation_index
FROM wallets w
WHERE w.id = pm.wallet_id;

ALTER TABLE store_payment_methods ALTER COLUMN xpub SET NOT NULL;
ALTER TABLE store_payment_methods DROP COLUMN wallet_id;

DROP TABLE IF EXISTS store_wallets;
DROP TABLE IF EXISTS wallets;
