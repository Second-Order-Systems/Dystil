CREATE TABLE IF NOT EXISTS ai_keys (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL,
    key_prefix TEXT NOT NULL UNIQUE,
    key_hash TEXT NOT NULL UNIQUE,
    spend_limit_microusd BIGINT NOT NULL CHECK (spend_limit_microusd > 0),
    spent_microusd BIGINT NOT NULL DEFAULT 0 CHECK (spent_microusd >= 0),
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS ai_usage (
    id UUID PRIMARY KEY,
    key_id UUID NOT NULL REFERENCES ai_keys(id),
    openai_request_id TEXT,
    model TEXT NOT NULL,
    input_tokens BIGINT NOT NULL CHECK (input_tokens >= 0),
    cached_input_tokens BIGINT NOT NULL DEFAULT 0 CHECK (cached_input_tokens >= 0),
    output_tokens BIGINT NOT NULL CHECK (output_tokens >= 0),
    cost_microusd BIGINT NOT NULL CHECK (cost_microusd >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_ai_usage_key_created_at
ON ai_usage (key_id, created_at DESC);
