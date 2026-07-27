CREATE TABLE IF NOT EXISTS memory_segments (
    org_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    segment_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    device_sequence BIGINT NOT NULL,
    previous_segment_id TEXT,
    start_time TIMESTAMPTZ NOT NULL,
    end_time TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ NOT NULL,
    segmenter_version TEXT NOT NULL,
    evidence_version TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    token_estimate INTEGER NOT NULL,
    audio_state TEXT NOT NULL,
    envelope_json JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'ready',
    priority INTEGER NOT NULL DEFAULT 100,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    leased_by TEXT,
    leased_until TIMESTAMPTZ,
    fencing_token TEXT,
    last_error TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    superseded_at TIMESTAMPTZ,
    PRIMARY KEY (org_id, device_id, segment_id, revision),
    UNIQUE (org_id, device_id, device_sequence, revision)
);

CREATE INDEX IF NOT EXISTS idx_memory_segments_user_timeline
ON memory_segments (org_id, user_id, start_time, device_id, device_sequence);

CREATE INDEX IF NOT EXISTS idx_memory_segments_episode_queue
ON memory_segments (status, available_at, priority, received_at);

CREATE INDEX IF NOT EXISTS idx_memory_segments_envelope
ON memory_segments USING gin (envelope_json);
