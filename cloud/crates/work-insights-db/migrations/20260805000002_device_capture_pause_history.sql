-- Historical pause telemetry is intentionally separate from devices, which
-- remains the source of truth for a device's current capture state.
CREATE TABLE IF NOT EXISTS device_capture_pauses (
    id BIGSERIAL PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES app_users(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    paused_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    scheduled_resume_at TIMESTAMPTZ NOT NULL,
    resumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A device can only be in one pause session at a time. This makes retrying
-- capture-state reconciliation safe without requiring a client event ID.
CREATE UNIQUE INDEX IF NOT EXISTS idx_device_capture_pauses_one_open_per_device
ON device_capture_pauses (device_id)
WHERE resumed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_device_capture_pauses_org_user_paused_at
ON device_capture_pauses (org_id, user_id, paused_at DESC);
