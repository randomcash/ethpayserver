-- Sepolia's stablecoins, so a testnet payment keeps its symbol.
--
-- `payments.asset_symbol` is resolved by looking the contract up in this
-- table. Every mainnet chain has its stablecoins seeded and Sepolia has
-- nothing, so an ERC-20 payment on the one chain we actually test on fell
-- through to the unknown-token branch and was stored under a made-up symbol.
--
-- That is not only cosmetic. The symbol is what the analytics reader groups
-- by and what the plugin volume capability prices against, so a payment
-- stored without a resolvable one is invisible to the pricing ladder - which
-- is how the first real USDC payment on this instance came out worth nothing.
--
-- Circle's own Sepolia deployment. Chain 11155111.
INSERT INTO tokens (token_type, chain_id, address, symbol, name, decimals) VALUES
    ('erc20', 11155111, '0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238', 'USDC', 'USD Coin', 6)
ON CONFLICT DO NOTHING;
