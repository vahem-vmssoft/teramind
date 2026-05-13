CREATE TABLE agents (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind         text NOT NULL,
  version      text,
  installed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (kind, version)
);
