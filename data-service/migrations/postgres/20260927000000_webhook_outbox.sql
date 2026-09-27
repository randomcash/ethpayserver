-- Transactional outbox for webhook notification obligations.
--
-- A payment row and its webhook obligation used to be two independent
-- writes: the payment committed, then a separate call built and queued a
-- webhook payload. A crash between the two left a payment on file with no
-- obligation ever recorded, and nothing to replay it from. This table is
-- written in the same database transaction as the payment row (see
-- `PaymentTxIndexWriter::upsert_with_tx_index_and_obligation`), so the two
-- either both commit or neither does. A background drain reads rows where
-- `dispatched_at IS NULL`, turns each into an actual queued delivery, and
-- marks it dispatched - at-least-once, same as delivery itself.
--
-- `payment_id` is `NOT NULL` and cascades on delete: an obligation that
-- cannot name the payment it is about cannot be audited, which is the gap
-- `webhook_deliveries` has and this table does not repeat.
CREATE TABLE webhook_outbox (
    id              UUID PRIMARY KEY,
    payment_id      UUID NOT NULL REFERENCES payments(id) ON DELETE CASCADE,
    invoice_id      TEXT NOT NULL,
    event_type      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    dispatched_at   TIMESTAMPTZ,
    -- A redelivered monitor event (delivery is documented as at-least-once)
    -- re-runs the same payment upsert and would otherwise write a second
    -- obligation for a payment already dispatched or awaiting dispatch.
    UNIQUE (payment_id, event_type)
);

-- The drain reads exactly this set, oldest first.
CREATE INDEX idx_webhook_outbox_undispatched
    ON webhook_outbox (created_at)
    WHERE dispatched_at IS NULL;
