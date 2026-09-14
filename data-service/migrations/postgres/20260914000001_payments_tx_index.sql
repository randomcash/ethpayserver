-- One transaction can pay two different watched addresses - a batching
-- contract, a multicall, an exchange sweep, or a merchant settling two
-- invoices at once. `unique_payment_tx` was `(tx_hash, chain_id)` only, with
-- no way to tell those two transfers apart, so `PgDataService::upsert`'s
-- `ON CONFLICT (tx_hash, chain_id) DO UPDATE` collapsed the second transfer
-- into the first and silently discarded it.
--
-- `tx_index` is the EVM log index of the transfer within its transaction for
-- an ERC20 transfer, or a fixed -1 sentinel for a native transfer (see
-- payment_handler.rs for why -1 rather than 0: it must never collide with a
-- real log index of 0 in the same transaction). Defaulting existing rows to 0
-- keeps every one of them - all genuinely one transfer per transaction so far
-- - a trivial backfill: nothing to reconcile, nothing to look up. New rows
-- never write 0 for a native transfer going forward, but the column stays a
-- plain signed integer so both this backfill value and the -1 sentinel fit.
ALTER TABLE payments ADD COLUMN tx_index INTEGER NOT NULL DEFAULT 0;

ALTER TABLE payments DROP CONSTRAINT unique_payment_tx;
ALTER TABLE payments ADD CONSTRAINT unique_payment_tx UNIQUE (chain_id, tx_hash, tx_index);

COMMENT ON COLUMN payments.tx_index IS
    'Log index of this transfer within its transaction, or -1 for a native '
    'transfer. Distinguishes two transfers to different watched addresses '
    'batched into one transaction, which otherwise share (chain_id, tx_hash).';
