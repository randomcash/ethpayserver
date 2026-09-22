-- Stablecoins (and, where unambiguous, WETH/WBTC) for the chains that had
-- zero rows in `tokens`: zkSync Era, Linea and Scroll mainnet, plus the
-- Sepolia-family testnets that `evm::testnet::ALL_TESTNETS` already offers on
-- the settings page but whose payments resolved to a truncated address
-- instead of a symbol.
--
-- Every address below was checked against the issuer's or chain operator's
-- own source, not a block explorer search or another repository's list:
-- Circle's published USDC contract-address list for every USDC row below,
-- and each L2's own official token list (Consensys' linea-token-list,
-- Scroll's token-list) or block-explorer API (Matter Labs', for zkSync Era)
-- for bridged assets - cross-checked so the bridged token's `l1Address` /
-- `rootAddress` is the genuine mainnet contract and not a same-symbol
-- impostor squatting the explorer.
--
-- Left out, deliberately, rather than guessed:
--  - zkSync Era WETH: the chain has two live, non-scam WETH contracts (one
--    bridged, one a separately deployed L2-native wrapper) and no official
--    source picks either as canonical.
--  - Holesky, Hoodi and BSC testnet: no Circle or Tether testnet deployment
--    exists on any of the three, per Circle's own supported-network list.
--
-- `chain_id` is `caip2`, so `eip155:<id>` and not a bare integer - see
-- 20260908140000_caip2_chain_identity.sql.
INSERT INTO tokens (token_type, chain_id, address, symbol, name, decimals) VALUES
    -- zkSync Era (eip155:324): native Circle USDC. USDT and WBTC are the
    -- standard-bridge representations, verified by confirming the explorer's
    -- l1Address for each is the genuine Ethereum mainnet contract.
    ('erc20', 'eip155:324', '0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:324', '0x493257fD37EDB34451f62EDf8D2a0C418852bA4C', 'USDT', 'Tether USD', 6),
    ('erc20', 'eip155:324', '0xBBeB516fb02a01611cBBE0453Fe3c580D7281011', 'WBTC', 'Wrapped BTC', 8),

    -- Linea (eip155:59144): native Circle USDC; USDT/WBTC/WETH are Linea's
    -- own canonical-bridge token list.
    ('erc20', 'eip155:59144', '0x176211869cA2b568f2A7D4EE941E073a821EE1ff', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:59144', '0xA219439258ca9da29E9Cc4cE5596924745e12B93', 'USDT', 'Tether USD', 6),
    ('erc20', 'eip155:59144', '0x3aAB2285ddcDdaD8edf438C1bAB47e1a9D05a9b4', 'WBTC', 'Wrapped BTC', 8),
    ('erc20', 'eip155:59144', '0xe5D7C2a44FfDDf6b295A15c148167daaAf5Cf34f', 'WETH', 'Wrapped Ether', 18),

    -- Scroll (eip155:534352): Circle has no native USDC here, so USDC below
    -- is the canonical-bridge representation per Scroll's own token list -
    -- same source for USDT and WBTC. WETH is Scroll's predeployed canonical
    -- wrapper at a reserved system address, not a bridged token.
    ('erc20', 'eip155:534352', '0x06eFdBFf2a14a7c8E15944D1F4A48F9F95F663A4', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:534352', '0xf55BEC9cafDbE8730f096Aa55dad6D22d44099Df', 'USDT', 'Tether USD', 6),
    ('erc20', 'eip155:534352', '0x3C1BCa5a656e69edCD0D4E36BEbb3FcDAcA60Cf1', 'WBTC', 'Wrapped BTC', 8),
    ('erc20', 'eip155:534352', '0x5300000000000000000000000000000000000004', 'WETH', 'Wrapped Ether', 18),

    -- Testnets: Circle's official USDC test deployments, for the rest of
    -- `ALL_TESTNETS` that Circle actually supports. Sepolia itself already
    -- has USDC from 20260920000000_sepolia_tokens.sql.
    ('erc20', 'eip155:11155420', '0x5fd84259d66Cd46123540766Be93DFE6D43130D7', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:421614', '0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:84532', '0x036CbD53842c5426634e7929541eC2318f3dCF7e', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:43113', '0x5425890298aed601595a70AB815c96711a31Bc65', 'USDC', 'USD Coin', 6),
    ('erc20', 'eip155:80002', '0x41E94Eb019C0762f9Bfcf9Fb1E58725BfB0e7582', 'USDC', 'USD Coin', 6)
ON CONFLICT DO NOTHING;
