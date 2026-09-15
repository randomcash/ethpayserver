-- Reversible only while no tx_hash/chain_id pair has more than one tx_index in
-- use - once two real batched transfers exist, collapsing back to
-- (tx_hash, chain_id) would recreate the exact bug this migration fixes.
DO $$
DECLARE collapsed INTEGER;
BEGIN
    SELECT COUNT(*) INTO collapsed
    FROM (
        SELECT chain_id, tx_hash
        FROM payments
        GROUP BY chain_id, tx_hash
        HAVING COUNT(DISTINCT tx_index) > 1
    ) AS batched;

    IF collapsed > 0 THEN
        RAISE EXCEPTION
            '% (chain_id, tx_hash) pair(s) have more than one tx_index. '
            'Rolling back would collapse them onto a single payment again.',
            collapsed;
    END IF;
END $$;

ALTER TABLE payments DROP CONSTRAINT unique_payment_tx;
ALTER TABLE payments ADD CONSTRAINT unique_payment_tx UNIQUE (tx_hash, chain_id);

ALTER TABLE payments DROP COLUMN tx_index;
