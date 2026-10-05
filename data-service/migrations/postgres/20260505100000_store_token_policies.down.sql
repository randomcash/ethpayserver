DROP TRIGGER IF EXISTS trg_store_token_policies_updated ON store_token_policies;
DROP FUNCTION IF EXISTS update_store_token_policy_timestamp();
DROP TABLE IF EXISTS store_token_policy_entries;
DROP TABLE IF EXISTS store_token_policies;
