-- Pending email changes awaiting verification.
--
-- Changing `users.email` directly would leave the address pending with no
-- proof its owner can read mail there. This table holds the request until a
-- single-use, short-lived token proves that, then the caller applies it via
-- the ordinary `UserRepository::update_user` path - which already tolerates
-- changing `email` while leaving `kdf_salt_identifier` untouched, the
-- immutable field the recovery KDF is salted with at registration.
--
-- At most one unconsumed request per user, enforced by the unique index
-- below rather than only in application code. A plain (non-unique) index
-- plus a DELETE-then-INSERT in the writer looked sufficient but was not: two
-- concurrent requests for the same user (a double-submitted click, or a
-- resubmission before the first request's transaction commits) could each
-- see nothing to delete and both insert, leaving two live tokens for
-- different addresses. `EmailChangeWriter::create_email_change_request` uses
-- `INSERT ... ON CONFLICT ... DO UPDATE` against this constraint instead,
-- which is a single atomic statement with no such window - so a stale link
-- from an earlier attempt can never resurrect a superseded email even under
-- concurrent resubmission.

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

-- Both the caller's own-id lookup and the "at most one" guarantee: UNIQUE
-- makes the invariant a database constraint, not just an application-level
-- convention two concurrent writers could race past.
CREATE UNIQUE INDEX idx_email_change_requests_user_id
    ON email_change_requests(user_id) WHERE consumed_at IS NULL;

COMMENT ON TABLE email_change_requests IS
    'Pending email changes awaiting verification. At most one unconsumed row '
    'per user; a token is redeemed at most once and only before expires_at.';
