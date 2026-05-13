CREATE TABLE tool_calls (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  turn_id      uuid NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  ordinal      integer NOT NULL,
  name         text NOT NULL,
  input        jsonb NOT NULL,
  output       text,
  is_error     boolean NOT NULL DEFAULT false,
  started_at   timestamptz NOT NULL,
  duration_ms  integer,
  UNIQUE (turn_id, ordinal)
);
CREATE INDEX tool_calls_turn ON tool_calls (turn_id, ordinal);
CREATE INDEX tool_calls_name ON tool_calls (name);
