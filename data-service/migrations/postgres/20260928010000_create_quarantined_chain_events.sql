-- Events the consumer could never apply (an invoice since deleted, a
-- malformed token field), skipped so one bad envelope cannot stop every
-- merchant behind it. Skipping is only acceptable if the skip is findable:
-- a row here is written before the cursor moves past the event, and holds
-- the whole payload so an operator can re-credit a payment by hand.
CREATE TABLE quarantined_chain_events (
    id BIGSERIAL PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    epoch BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    reason TEXT NOT NULL,
    event JSONB NOT NULL,
    quarantined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Redelivery of the same envelope must not add a second row.
    UNIQUE (adapter_id, chain_id, epoch, seq)
);
