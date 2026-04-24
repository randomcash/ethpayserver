-- Add deprecation support for API key rotation (RCS-102).
-- deprecated_at is set when a key is rotated; after the grace window the old key stops authenticating.
ALTER TABLE api_keys ADD COLUMN deprecated_at TIMESTAMPTZ;
