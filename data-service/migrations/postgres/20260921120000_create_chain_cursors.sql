-- The orchestrator's durable resume point into an adapter's event outbox.
--
-- One row per (adapter_id, chain_id): `adapter_id` identifies the event
-- source (today, always the evmmonitor deployment), `chain_id` is the raw
-- EIP-155 id used throughout `watched_addresses` and `MonitorEvent`.
--
-- `epoch` names which outbox lineage `seq` belongs to - a fresh lineage
-- starts whenever the adapter can no longer vouch for continuity of its own
-- outbox, and a `seq` compared against the wrong lineage is meaningless.
-- `block_height` is the chain height as of `seq`, carried for diagnostics.
CREATE TABLE chain_cursors (
    adapter_id TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    epoch BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    block_height BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (adapter_id, chain_id)
);
