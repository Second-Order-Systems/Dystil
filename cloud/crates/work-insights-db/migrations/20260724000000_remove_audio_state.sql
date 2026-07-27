-- Dystil schema v3 has screen/input evidence only. Historical envelope_json
-- remains untouched; this removes the redundant audio-only index column from
-- all future writes.
ALTER TABLE memory_segments DROP COLUMN IF EXISTS audio_state;
