-- Correct payment rows written by the truncated-address fallback.
--
-- Before ethpayserver#215, an ERC20 payment whose token wasn't in `tokens`
-- was recorded with `asset_symbol` set to `format!("0x{}...", &addr[2..8])`
-- - six hex digits and an ellipsis, not an identifier. #215 fixed the code
-- for new payments and seeded Sepolia's USDC, but a row already written this
-- way stays wrong until something rewrites it, which is what this migration
-- does.
--
-- `asset_symbol` is read by machines, not just displayed: the analytics
-- reader groups by it and the volume capability prices against it, so a row
-- stuck on an address fragment is invisible to both.
--
-- Resolved the same way #215's fallback now resolves a new payment: look the
-- contract up in `tokens` by (chain_id, address), or fall back to `ERC20` if
-- it still isn't there - never derive a symbol from the address itself, which
-- is the bug this migration exists to undo. The address comparison is
-- case-insensitive because `payments.token_address` is stored lowercase
-- (`format!("{:#x}", ...)`) while `tokens.address` is checksummed.
UPDATE payments p
SET asset_symbol = COALESCE(
    (
        SELECT t.symbol
        FROM tokens t
        WHERE t.chain_id = p.chain_id
          AND LOWER(t.address) = LOWER(p.token_address)
        LIMIT 1
    ),
    'ERC20'
)
WHERE p.token_address IS NOT NULL
  AND p.asset_symbol ~* '^0x[0-9a-f]{6}\.\.\.$';

-- payment_options.asset_symbol is copied from a merchant's own
-- store_payment_methods.asset_symbol at invoice-option creation time
-- (server/src/api/invoices/payment_options.rs), which is merchant-supplied
-- input, not derived from an address - so the write path this migration
-- exists for has never touched this table. Still applying the identical fix
-- here rather than trusting that by inspection: the column has the same
-- shape (chain_id, token_address) and if any row ever did end up in the
-- truncated-address form, by whatever path, this makes the table correct
-- rather than leaving it to another migration later.
UPDATE payment_options po
SET asset_symbol = COALESCE(
    (
        SELECT t.symbol
        FROM tokens t
        WHERE t.chain_id = po.chain_id
          AND LOWER(t.address) = LOWER(po.token_address)
        LIMIT 1
    ),
    'ERC20'
)
WHERE po.token_address IS NOT NULL
  AND po.asset_symbol ~* '^0x[0-9a-f]{6}\.\.\.$';
