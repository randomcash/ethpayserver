-- Add Fantom Opera and Gnosis Chain support

-- Chain configurations
INSERT INTO chain_configs (chain_id, name, native_symbol, native_decimals, explorer_url, required_confirmations) VALUES
    (250, 'Fantom Opera', 'FTM', 18, 'https://ftmscan.com', 12),
    (100, 'Gnosis Chain', 'xDAI', 18, 'https://gnosisscan.io', 20);

-- Common stablecoins on Fantom (chain_id = 250)
INSERT INTO tokens (token_type, chain_id, address, symbol, name, decimals) VALUES
    ('erc20', 250, '0x049d68029688eAbF473097a2fC38ef61633A3C7A', 'fUSDT', 'Frapped USDT', 6),
    ('erc20', 250, '0x04068DA6C83AFCFA0e13ba15A6696662335D5B75', 'USDC', 'USD Coin', 6),
    ('erc20', 250, '0x8D11eC38a3EB5E956B052f67Da8Bdc9bef8Abf3E', 'DAI', 'Dai Stablecoin', 18);

-- Common stablecoins on Gnosis (chain_id = 100)
INSERT INTO tokens (token_type, chain_id, address, symbol, name, decimals) VALUES
    ('erc20', 100, '0x4ECaBa5870353805a9F068101A40E0f32ed605C6', 'USDT', 'Tether USD', 6),
    ('erc20', 100, '0xDDAfbb505ad214D7b80b1f830fcCc89B60fb7A83', 'USDC', 'USD Coin', 6),
    ('erc20', 100, '0xe91D153E0b41518A2Ce8Dd3D7944Fa863463a97d', 'WXDAI', 'Wrapped XDAI', 18);
