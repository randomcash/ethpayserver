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
-- and then re-pointing the native ones at -1 keeps the backfill trivial -
-- nothing to reconcile against chain data - while still landing every
-- existing row on the same value a recomputation of that same transfer would
-- produce going forward: native rows already used a made-up 0 that never came
-- from a real log index, so moving them to -1 costs nothing, and ERC20 rows
-- keep 0, correct for the overwhelming majority (one transfer per
-- transaction) and merely approximate - not recoverable from this table
-- alone - for the rest.
ALTER TABLE payments ADD COLUMN tx_index INTEGER NOT NULL DEFAULT 0;

UPDATE payments SET tx_index = -1 WHERE asset_type = 'native';

ALTER TABLE payments DROP CONSTRAINT unique_payment_tx;
ALTER TABLE payments ADD CONSTRAINT unique_payment_tx UNIQUE (chain_id, tx_hash, tx_index);

COMMENT ON COLUMN payments.tx_index IS
    'Log index of this transfer within its transaction, or -1 for a native '
    'transfer. Distinguishes two transfers to different watched addresses '
    'batched into one transaction, which otherwise share (chain_id, tx_hash).';
