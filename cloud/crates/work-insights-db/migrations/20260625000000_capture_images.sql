CREATE TABLE IF NOT EXISTS capture_images (
    org_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    image_id TEXT NOT NULL,
    client_image_key TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    object_key TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    byte_size BIGINT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    selection_reason TEXT NOT NULL,
    status TEXT NOT NULL,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (org_id, image_id),
    UNIQUE (org_id, user_id, device_id, client_image_key)
);

CREATE INDEX IF NOT EXISTS idx_capture_images_status
ON capture_images (org_id, status, updated_at);
