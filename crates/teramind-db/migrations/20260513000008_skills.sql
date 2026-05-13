CREATE TABLE skills (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name         text NOT NULL UNIQUE,
  description  text NOT NULL,
  body         text NOT NULL,
  source       text NOT NULL CHECK (source IN ('authored','codified','imported')),
  source_session_ids uuid[] NOT NULL DEFAULT '{}',
  created_at   timestamptz NOT NULL DEFAULT now(),
  updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX skills_name_trgm ON skills USING gin (name gin_trgm_ops);
CREATE INDEX skills_body_trgm ON skills USING gin (body gin_trgm_ops);
