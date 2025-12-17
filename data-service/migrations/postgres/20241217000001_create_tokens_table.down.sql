-- Rollback tokens table

DROP TRIGGER IF EXISTS tokens_updated_at ON tokens;
DROP FUNCTION IF EXISTS update_tokens_updated_at();
DROP TABLE IF EXISTS tokens;
