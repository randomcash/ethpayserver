-- Remove Fantom and Gnosis chain support
DELETE FROM tokens WHERE chain_id IN (250, 100);
DELETE FROM chain_configs WHERE chain_id IN (250, 100);
