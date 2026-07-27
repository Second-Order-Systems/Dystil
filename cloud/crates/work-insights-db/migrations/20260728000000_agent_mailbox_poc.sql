CREATE TABLE agent_messages (
    sequence_id BIGSERIAL PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    sender_user_id TEXT NOT NULL REFERENCES app_users(id) ON DELETE CASCADE,
    sender_device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    recipient_user_id TEXT NOT NULL REFERENCES app_users(id) ON DELETE CASCADE,
    recipient_device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    in_reply_to TEXT REFERENCES agent_messages(message_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('request', 'status', 'response', 'error')),
    turn_index SMALLINT NOT NULL CHECK (turn_index >= 0 AND turn_index <= 1),
    body_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX agent_messages_recipient_cursor
ON agent_messages (recipient_device_id, sequence_id);

CREATE INDEX agent_messages_expiry
ON agent_messages (expires_at);

CREATE INDEX agent_messages_request_rate_limit
ON agent_messages (sender_user_id, recipient_user_id, created_at)
WHERE kind = 'request';
