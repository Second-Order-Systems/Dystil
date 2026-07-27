CREATE TABLE IF NOT EXISTS organization_sync_policies (
    org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    schema_version INTEGER NOT NULL,
    policy_version TEXT NOT NULL,
    policy_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by TEXT
);

ALTER TABLE capture_images
    ADD COLUMN IF NOT EXISTS sync_metadata JSONB;

ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS app_version TEXT,
    ADD COLUMN IF NOT EXISTS build_channel TEXT,
    ADD COLUMN IF NOT EXISTS build_commit TEXT,
    ADD COLUMN IF NOT EXISTS sync_capabilities TEXT[],
    ADD COLUMN IF NOT EXISTS version_reported_at TIMESTAMPTZ;
