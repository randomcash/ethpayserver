-- Add per-API-key rate limit column.
-- NULL means "use server default" (RATE_LIMIT_API_KEY_DEFAULT env, default 300 rpm).
ALTER TABLE api_keys ADD COLUMN rate_limit_rpm INT NULL;
