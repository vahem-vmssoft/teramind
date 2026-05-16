-- Team-mode tables and additive columns on sessions/skills.
-- Additive only; safe to apply to local-first installs.

CREATE TABLE users (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  email        text NOT NULL UNIQUE,
  display_name text,
  created_at   timestamptz NOT NULL DEFAULT now(),
  revoked_at   timestamptz
);

CREATE TABLE devices (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         text NOT NULL,
  token_hash   bytea NOT NULL UNIQUE,
  public_key   bytea NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz,
  revoked_at   timestamptz
);
CREATE INDEX devices_user      ON devices (user_id);
CREATE INDEX devices_last_seen ON devices (last_seen_at DESC);

CREATE TABLE invites (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code_hash       bytea NOT NULL UNIQUE,
  invited_email   text NOT NULL,
  display_name    text,
  created_by      text,
  created_at      timestamptz NOT NULL DEFAULT now(),
  expires_at      timestamptz NOT NULL,
  redeemed_at     timestamptz,
  redeemed_device uuid REFERENCES devices(id)
);
CREATE INDEX invites_email   ON invites (invited_email);
CREATE INDEX invites_expires ON invites (expires_at) WHERE redeemed_at IS NULL;

ALTER TABLE sessions ADD COLUMN user_id   uuid REFERENCES users(id);
ALTER TABLE sessions ADD COLUMN device_id uuid REFERENCES devices(id);
ALTER TABLE skills   ADD COLUMN user_id   uuid REFERENCES users(id);
ALTER TABLE skills   ADD COLUMN device_id uuid REFERENCES devices(id);

CREATE INDEX sessions_user        ON sessions (user_id);
CREATE INDEX sessions_user_recent ON sessions (user_id, started_at DESC);
CREATE INDEX skills_user          ON skills (user_id);
