CREATE TABLE projects (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  root_path    text NOT NULL UNIQUE,
  git_remote   text,
  display_name text,
  first_seen   timestamptz NOT NULL DEFAULT now()
);
