-- Pending email changes awaiting verification (RCS-263).
--
-- Changing `users.email` directly would leave the address pending with no
-- proof its owner can read mail there. This table holds the request until a
-- single-use, short-lived token proves that, then the caller applies it via
-- the ordinary `UserRepository::update_user` path - which already tolerates
-- changing `email` while leaving `kdf_salt_identifier` untouched, the
-- immutable field the recovery KDF is salted with at registration.
--
-- At most one unconsumed request per user: starting a new one deletes any
-- prior unconsumed row for that user first (see `EmailChangeWriter`), so a
-- stale link from an earlier attempt can never resurrect a superseded email.

CREATE TABLE email_change_requests (
    token UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    new_email VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    -- NULL until the token is redeemed. Checked, never just relied on for
    -- cleanup, so a token cannot be replayed after it has already taken
    -- effect.
    consumed_at TIMESTAMPTZ
);

-- Only ever looked up by the caller's own id (to invalidate a prior request)
-- while unconsumed; consumed rows are dead weight for that query.
CREATE INDEX idx_email_change_requests_user_id
    ON email_change_requests(user_id) WHERE consumed_at IS NULL;

COMMENT ON TABLE email_change_requests IS
    'Pending email changes awaiting verification. At most one unconsumed row '
    'per user; a token is redeemed at most once and only before expires_at.';
