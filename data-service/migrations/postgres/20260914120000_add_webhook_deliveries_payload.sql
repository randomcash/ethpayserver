-- The delivery path is about to start writing to `webhook_deliveries`, and
-- replay needs the exact payload that was queued, not a snapshot of the
-- invoice's current state (which may have moved on since). The table has no
-- reader or writer yet (see the migration that created it), so this is a
-- plain additive column, not a backfill.

ALTER TABLE webhook_deliveries ADD COLUMN IF NOT EXISTS payload JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE webhook_deliveries ALTER COLUMN payload DROP DEFAULT;
