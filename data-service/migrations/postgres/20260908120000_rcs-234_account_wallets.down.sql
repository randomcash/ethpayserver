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

-- Which wallet each payment method actually derives from, resolved once.
--
-- Both guards and the reversal itself need this, and an earlier version had
-- them disagree: the guards looked only at `store_payment_methods.wallet_id`
-- while the reversal also walked the store override and the account primary.
-- A wallet shared by three INHERITING methods therefore passed the "shared"
-- guard - that column is NULL on all three - and the reversal then copied one
-- index onto all three, which is the precise outcome the guard exists to
-- refuse. `set_store_wallet` NULLs every method in a store, so that is not an
-- exotic state; it is what using the per-store override at all produces.
--
-- ON COMMIT DROP: the migration is one transaction, and the connection is
-- pooled and reused.
CREATE TEMP TABLE rcs234_down_effective ON COMMIT DROP AS
SELECT pm.id AS payment_method_id,
       COALESCE(
           pm.wallet_id,
           (SELECT sw.wallet_id FROM store_wallets sw WHERE sw.store_id = pm.store_id),
           (SELECT p.id FROM wallets p WHERE p.user_id = s.owner_id AND p.is_primary)
       ) AS wallet_id
FROM store_payment_methods pm
JOIN stores s ON s.id = pm.store_id;

DO $$
DECLARE shared INTEGER;
DECLARE stranded INTEGER;
DECLARE unresolved INTEGER;
BEGIN
    SELECT COUNT(*) INTO shared FROM (
        SELECT wallet_id FROM rcs234_down_effective
        WHERE wallet_id IS NOT NULL
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

    -- A counter with nowhere to land is just as dangerous as a merged one, and
    -- less obvious. The reversal below copies each wallet's index back onto the
    -- payment method that derives from it; a wallet at index 50 whose method
    -- was deleted has none, would be dropped with the table, and the next time
    -- that xpub is added it starts at 0 and re-issues 0..49 - addresses that
    -- may already hold funds.
    --
    -- Resolved, not pinned. A wallet reached only through a store override has
    -- somewhere perfectly good to land, and refusing it would block a rollback
    -- this migration can in fact perform losslessly.
    --
    -- Wallets still at 0 have issued nothing and are safe to lose.
    SELECT COUNT(*) INTO stranded
    FROM wallets w
    WHERE w.derivation_index > 0
      AND NOT EXISTS (
          SELECT 1 FROM rcs234_down_effective e WHERE e.wallet_id = w.id
      );

    IF stranded > 0 THEN
        RAISE EXCEPTION
            'RCS-234 down: % wallet(s) have issued addresses but no payment '
            'method to carry their derivation index back to. Reverting would '
            'drop the counter, and re-adding the xpub would start at 0 and '
            're-issue every address it has already produced. Restore from a '
            'pre-migration backup instead.', stranded;
    END IF;

    -- The mirror image: a method whose resolution chain runs out. It is pinned
    -- to nothing, its store has no override, and its account has no primary -
    -- reachable by deleting a primary wallet that nothing referenced. There is
    -- no xpub to put back, and the old schema had the column NOT NULL, so this
    -- would surface as a bare constraint violation several statements later.
    SELECT COUNT(*) INTO unresolved
    FROM rcs234_down_effective WHERE wallet_id IS NULL;

    IF unresolved > 0 THEN
        RAISE EXCEPTION
            'RCS-234 down: % payment method(s) resolve to no wallet at all, so '
            'there is no xpub to restore onto them. The old schema requires '
            'one. Give the account a primary wallet, or restore from a '
            'pre-migration backup.', unresolved;
    END IF;
END $$;

-- Losses on the way back, stated rather than hidden:
--   * per-store overrides (store_wallets) - the old shape had no such concept.
--   * wallet names, is_primary, and any wallet not referenced by a payment
--     method - likewise no home in the old shape.
--   * payment_options provenance (wallet_id, derivation_index).
--   * the partial unique index closing the native-asset NULL gap. Duplicate
--     native methods collapsed on the way up are NOT restored - they were
--     duplicates of each other, and recreating them would recreate the
--     duplicate counters this ticket removed.
-- Addresses already issued are unaffected: payment_address is untouched.

ALTER TABLE payment_options DROP COLUMN IF EXISTS derivation_index;
ALTER TABLE payment_options DROP COLUMN IF EXISTS wallet_id;

ALTER TABLE store_payment_methods ADD COLUMN xpub VARCHAR(120);
ALTER TABLE store_payment_methods ADD COLUMN derivation_index INTEGER NOT NULL DEFAULT 0;

-- One statement, through the same resolution the guards checked - pinned
-- methods and inheriting ones alike. Two statements with two spellings of the
-- chain is how the guards and the reversal drifted apart in the first place.
UPDATE store_payment_methods pm
SET xpub = w.xpub, derivation_index = w.derivation_index
FROM rcs234_down_effective e
JOIN wallets w ON w.id = e.wallet_id
WHERE pm.id = e.payment_method_id;

DROP INDEX IF EXISTS idx_store_payment_methods_native;

ALTER TABLE store_payment_methods ALTER COLUMN xpub SET NOT NULL;
ALTER TABLE store_payment_methods DROP COLUMN wallet_id;

DROP TABLE IF EXISTS store_wallets;
DROP TABLE IF EXISTS wallets;
