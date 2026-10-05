-- A wallet belongs to a chain family, and every resolution is scoped by it.
--
-- `wallets.xpub` is a BIP-32 ACCOUNT-level key - `m/44'/60'/0'` - with the
-- BIP-44 coin type already baked in and the parent unreachable. Tron's coin
-- type is 195, not 60. So a key registered here carries the answer to "which
-- family is this for" and cannot be asked it.
--
-- Nor can it be told by looking. An Ethereum account xpub and a Tron one are
-- byte-indistinguishable: same base58 alphabet, same 0x0488B21E version bytes,
-- same length, both accepted by the same validator. A merchant pasting into
-- the wrong field gets no feedback, and neither does this server.
--
-- The consequence is not an error, it is a silent loss. Both families take the
-- last 20 bytes of the Keccak-256 hash of a secp256k1 public key, so running an
-- Ethereum account xpub through Tron's base58check encoding produces a valid,
-- checksum-correct `T...` address. Payments to it arrive. The merchant's Tron
-- wallet never shows them, because that wallet derives at `m/44'/195'`, and the
-- funds come back only by re-importing the seed at a non-standard path - which
-- most merchants cannot do and none should have to.
--
-- Recording the family is therefore not a classification convenience. It is the
-- only place the information exists at all, and once it is here, resolution can
-- refuse to answer with a wallet from the wrong family rather than derive an
-- unreachable address from it.

-- ---------------------------------------------------------------------------
-- 1. The namespace itself
-- ---------------------------------------------------------------------------
--
-- A CAIP-2 *namespace* - the part before the colon in `eip155:1` or
-- `tron:728126428`. A namespace, not a whole chain id, because a key is shared
-- by every chain in its family: one Ethereum xpub serves mainnet, Polygon and
-- Arbitrum alike, and storing `eip155:1` would make a wallet resolve for one of
-- them and not the others.
--
-- Its own domain rather than a bare TEXT, matching how `caip2` is already
-- spelled once in 20260908140000 rather than repeated at each column. This
-- table is written by migrations and by hand during incidents too, and
-- `Eip155` or `ethereum` in this column silently resolves for nothing - a
-- store that looks correctly configured and cannot take a payment.
CREATE DOMAIN caip2_namespace AS TEXT
    CHECK (VALUE ~ '^[-a-z0-9]{3,8}$');

COMMENT ON DOMAIN caip2_namespace IS
    'The namespace half of a CAIP-2 identifier - eip155, tron, solana. '
    'Lowercase, 3-8 characters of [-a-z0-9].';

-- DEFAULT 'eip155' is a statement about history, not a convenience. Every
-- wallet that exists when this runs was registered when EVM was the only
-- family this server had, so every one of them is an Ethereum key - the
-- default is the correct backfill for all of them, and there is no row it
-- could be wrong about.
--
-- It stays on the column afterwards so that an INSERT written before this
-- migration still succeeds. That is a deliberate trade and worth naming: a
-- future writer that forgets to bind a namespace files its key as Ethereum
-- instead of failing. The writer that matters (`upsert_wallet`) binds it
-- explicitly, the API refuses a request that names an unknown family, and
-- `POST /wallets` returns derived addresses for the merchant to check - so the
-- default is not the last line of defence.
ALTER TABLE wallets
    ADD COLUMN namespace caip2_namespace NOT NULL DEFAULT 'eip155';

COMMENT ON COLUMN wallets.namespace IS
    'CAIP-2 namespace this key derives for. Fixes the BIP-44 coin type the '
    'merchant exported it under (60 for eip155, 195 for tron) and the encoding '
    'of every address derived from it. Not recoverable from the xpub, which is '
    'why it is a column.';

-- ---------------------------------------------------------------------------
-- 2. One primary per family, not one per account
-- ---------------------------------------------------------------------------
--
-- The primary is the wallet a store falls back to when it has no override of
-- its own. An account collecting on two families needs one fallback in each: a
-- single account-wide primary hands every family the key of whichever family
-- happened to be registered first, which for the second family is precisely
-- the wrong-coin-type address described at the top of this file.
--
-- Dropped and recreated rather than widened in place - an index's column list
-- cannot be altered - and the two statements are in one implicit transaction,
-- so there is no window in which two primaries could be written.
DROP INDEX idx_account_wallets_one_primary;

CREATE UNIQUE INDEX idx_account_wallets_one_primary
    ON wallets(user_id, namespace) WHERE is_primary;

-- Every existing account has at most one primary and every existing wallet is
-- `eip155`, so no account gains or loses one here: the old index's guarantee is
-- exactly the new index's guarantee restricted to the only namespace present.

-- ---------------------------------------------------------------------------
-- 3. An xpub is unique within its family, not across families
-- ---------------------------------------------------------------------------
--
-- This one is the reason the ticket exists rather than a tidy-up that follows
-- from it. `create_wallet` documents, and relies on, "re-registering an xpub
-- the account already holds returns the existing row" - that is what stops a
-- second derivation counter appearing on one key. Keyed on (user_id, xpub)
-- alone, a merchant who registers their Tron xpub for Tron and their Ethereum
-- xpub for Ethereum is fine, but a merchant who registers the SAME bytes for
-- both - or whose two wallets happen to share a key - is handed back the
-- Ethereum row when they asked for Tron. Silently. Their Tron income then
-- derives at coin type 60.
--
-- With the namespace in the key, the same bytes in two families are two rows
-- with two counters. That is safe here in a way it is not within one family:
-- the addresses those counters produce live on different chains, so no
-- customer can be handed an address another customer already has.
DROP INDEX idx_account_wallets_user_xpub;

CREATE UNIQUE INDEX idx_account_wallets_user_xpub
    ON wallets(user_id, namespace, xpub);

-- Note what is deliberately NOT namespace-scoped: the cross-account refusal in
-- `reject_if_another_account_holds`. Two accounts sharing an xpub is either
-- co-custody or a mistake regardless of which families they registered it in,
-- and the check has no way to arbitrate one - so it stays keyed on the bytes.

-- ---------------------------------------------------------------------------
-- 4. A payment method cannot be pinned to a wallet from another family
-- ---------------------------------------------------------------------------
--
-- Resolution walks the method's pin, then the store's override, then the
-- account primary. Scoping the last two by namespace in SQL is straightforward
-- because both are looked up; the pin is not - it is a column already holding
-- a wallet id, and an eip155 wallet id sitting in `wallet_id` on a `tron:`
-- method would otherwise resolve perfectly happily.
--
-- Rather than filter that case out in every query that walks the chain (and
-- rely on all of them, forever, remembering to), make it unrepresentable. The
-- method's family is derivable from its chain id, so store it as a generated
-- column and let a composite foreign key demand that the wallet agrees.
--
-- MATCH SIMPLE - the default - is what makes this work alongside inheritance:
-- a composite foreign key with any NULL column is not enforced, and
-- `wallet_id` is NULL for exactly the methods that have no pin to check.
ALTER TABLE store_payment_methods
    ADD COLUMN chain_namespace caip2_namespace
        GENERATED ALWAYS AS (split_part(chain_id::text, ':', 1)) STORED;

COMMENT ON COLUMN store_payment_methods.chain_namespace IS
    'The chain family of chain_id, generated. Exists so a pinned wallet can be '
    'required to belong to the same family by foreign key rather than by every '
    'query remembering to check.';

-- The referenced side of a composite foreign key has to be unique, and
-- `wallets(id)` alone being the primary key is not enough for Postgres to
-- accept `(id, namespace)`.
CREATE UNIQUE INDEX idx_account_wallets_id_namespace ON wallets(id, namespace);

ALTER TABLE store_payment_methods
    ADD CONSTRAINT store_payment_methods_wallet_family_fkey
        FOREIGN KEY (wallet_id, chain_namespace)
        REFERENCES wallets(id, namespace) ON DELETE RESTRICT;

-- The single-column foreign key would now be redundant with the composite one,
-- which enforces strictly more. Dropped so there is one rule about what
-- `wallet_id` may hold, not two that could be reasoned about separately.
ALTER TABLE store_payment_methods
    DROP CONSTRAINT store_payment_methods_wallet_id_fkey;

-- ---------------------------------------------------------------------------
-- 5. A store override is per family
-- ---------------------------------------------------------------------------
--
-- `store_wallets` had `store_id` as its primary key, so a store could pin
-- exactly one wallet. With families that is not a limitation, it is a hazard:
-- pinning a store to a Tron wallet would REPLACE its Ethereum override, and
-- every EVM payment method that was following that override would silently
-- start resolving to the account's Ethereum primary instead - a change of
-- where real money is collected, caused by an action about a different chain.
--
-- So a store gets one override per family. `PUT /stores/{id}/wallet` with a
-- Tron wallet now says something about Tron and nothing about anything else.
ALTER TABLE store_wallets
    ADD COLUMN namespace caip2_namespace NOT NULL DEFAULT 'eip155';

-- Take it from the wallet rather than trusting the default. Every wallet is
-- `eip155` at this point so the two agree, but the column means "the family
-- this override is for" and the only authority on that is the row it points
-- at - a default that happened to be right once is not a backfill.
UPDATE store_wallets sw
SET namespace = w.namespace
FROM wallets w
WHERE w.id = sw.wallet_id;

ALTER TABLE store_wallets DROP CONSTRAINT store_wallets_pkey;
ALTER TABLE store_wallets ADD PRIMARY KEY (store_id, namespace);

-- Same composite foreign key as the payment methods get, and for the same
-- reason: an override recorded under `tron` that names an `eip155` wallet is
-- the original bug wearing a different hat.
ALTER TABLE store_wallets
    DROP CONSTRAINT store_wallets_wallet_id_fkey;

ALTER TABLE store_wallets
    ADD CONSTRAINT store_wallets_wallet_family_fkey
        FOREIGN KEY (wallet_id, namespace)
        REFERENCES wallets(id, namespace) ON DELETE RESTRICT;

COMMENT ON COLUMN store_wallets.namespace IS
    'Chain family this override applies to, always equal to the namespace of '
    'the wallet it names (enforced by foreign key). A store may hold one '
    'override per family and no more.';

COMMENT ON TABLE store_wallets IS
    'Per-store, per-family wallet override. No row for a family means stores '
    'on that family use the account primary for it.';
