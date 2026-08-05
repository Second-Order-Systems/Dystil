CREATE TABLE IF NOT EXISTS semantic_tree_samples (
    org_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    sample_id TEXT NOT NULL,
    source_frame_id BIGINT,
    surface_key TEXT NOT NULL,
    layout_fingerprint TEXT NOT NULL,
    schema_version SMALLINT NOT NULL,
    codec TEXT NOT NULL CHECK (codec = 'zstd'),
    payload_sha256 TEXT NOT NULL,
    payload BYTEA NOT NULL CHECK (octet_length(payload) <= 1048576),
    captured_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    platform TEXT NOT NULL,
    app_name TEXT NOT NULL,
    app_version TEXT,
    PRIMARY KEY (org_id, device_id, sample_id),
    UNIQUE (
        org_id,
        device_id,
        surface_key,
        layout_fingerprint,
        schema_version
    )
);

CREATE INDEX IF NOT EXISTS idx_semantic_tree_samples_surface
ON semantic_tree_samples (org_id, app_name, platform, surface_key, received_at DESC);
