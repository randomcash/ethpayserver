-- One transaction can pay two different watched addresses - a batching
-- contract, a multicall, an exchange sweep, or a merchant settling two
-- invoices at once. `unique_payment_tx` was `(tx_hash, chain_id)` only, with
-- no way to tell those two transfers apart, so `PgDataService::upsert`'s
-- `ON CONFLICT (tx_hash, chain_id) DO UPDATE` collapsed the second transfer
-- into the first and silently discarded it.
--
-- `tx_index` is the EVM log index of the transfer within its transaction (0
-- for a plain native transfer, which never shares a transaction with another
-- transfer to a watched address). Defaulting new rows to 0 keeps every
-- existing row - all genuinely one transfer per transaction so far - a
-- trivial backfill: nothing to reconcile, nothing to look up.
ALTER TABLE payments ADD COLUMN tx_index INTEGER NOT NULL DEFAULT 0;

ALTER TABLE payments DROP CONSTRAINT unique_payment_tx;
ALTER TABLE payments ADD CONSTRAINT unique_payment_tx UNIQUE (chain_id, tx_hash, tx_index);

COMMENT ON COLUMN payments.tx_index IS
    'Log index of this transfer within its transaction. Distinguishes two '
    'transfers to different watched addresses batched into one transaction, '
    'which otherwise share (chain_id, tx_hash).';
