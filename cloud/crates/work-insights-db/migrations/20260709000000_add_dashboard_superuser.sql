ALTER TABLE organizations
  ADD COLUMN IF NOT EXISTS superuser_user_id TEXT REFERENCES app_users(id);