ALTER TABLE ai_usage
ADD COLUMN IF NOT EXISTS cache_write_tokens BIGINT NOT NULL DEFAULT 0
CHECK (cache_write_tokens >= 0);
