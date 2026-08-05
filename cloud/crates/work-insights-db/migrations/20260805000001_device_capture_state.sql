ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS capture_state TEXT,
    ADD COLUMN IF NOT EXISTS capture_pause_until TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS capture_state_updated_at TIMESTAMPTZ;

ALTER TABLE devices
    DROP CONSTRAINT IF EXISTS devices_capture_state_check;

ALTER TABLE devices
    ADD CONSTRAINT devices_capture_state_check CHECK (
        (capture_state IS NULL AND capture_pause_until IS NULL)
        OR (capture_state = 'recording' AND capture_pause_until IS NULL)
        OR (capture_state = 'paused' AND capture_pause_until IS NOT NULL)
    );
