CREATE TABLE file_diffs (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  turn_id      uuid REFERENCES turns(id) ON DELETE CASCADE,
  session_id   uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  file_path    text NOT NULL,
  rel_path     text NOT NULL,
  attribution  text NOT NULL CHECK (attribution IN ('agent','human')),
  language     text,
  pre_excerpt  text NOT NULL,
  post_excerpt text NOT NULL,
  unified_diff text NOT NULL,
  pre_hash     bytea NOT NULL,
  post_hash    bytea NOT NULL,
  byte_size    integer NOT NULL,
  captured_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX file_diffs_session ON file_diffs (session_id, captured_at DESC);
CREATE INDEX file_diffs_relpath ON file_diffs (rel_path);
CREATE INDEX file_diffs_pre_excerpt_trgm ON file_diffs USING gin (pre_excerpt gin_trgm_ops);
CREATE INDEX file_diffs_post_excerpt_trgm ON file_diffs USING gin (post_excerpt gin_trgm_ops);
