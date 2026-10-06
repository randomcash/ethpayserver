-- Proof-of-possession challenges for promoting a wallet credential to
-- primary.
--
-- Distinct from `wallet_challenges` (the auth crate's own login-challenge
-- table, behind `ChallengeRepository`): reusing that table would let an
-- in-flight login challenge and an in-flight "make this wallet primary"
-- reauth challenge collide on the same single-row-per-user slot, and the two
-- ceremonies must not be answerable with each other's signature - solving a
-- login challenge must never be usable to authorize a credential swap, or
-- vice versa.
--
-- One row per user, same shape as `wallet_challenges`: a second challenge
-- request overwrites the first (`ON CONFLICT (user_id) DO UPDATE`), and
-- `DELETE ... RETURNING` in the statement that reads it makes each challenge
-- single-use.
CREATE TABLE wallet_reauth_challenges (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    challenge TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
