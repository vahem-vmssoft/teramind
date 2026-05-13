CREATE TABLE sessions (
  id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  agent_id          uuid NOT NULL REFERENCES agents(id),
  agent_session_id  text,
  cwd               text NOT NULL,
  project_id        uuid REFERENCES projects(id),
  parent_session_id uuid REFERENCES sessions(id),
  git_head          text,
  git_branch        text,
  os                text NOT NULL,
  hostname          text NOT NULL,
  user_login        text NOT NULL,
  started_at        timestamptz NOT NULL,
  ended_at          timestamptz,
  end_reason        text,
  metadata          jsonb NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX sessions_cwd_started ON sessions (cwd, started_at DESC);
CREATE INDEX sessions_project ON sessions (project_id, started_at DESC);
