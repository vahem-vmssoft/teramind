CREATE TABLE turns (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id      uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  ordinal         integer NOT NULL,
  started_at      timestamptz NOT NULL,
  ended_at        timestamptz,
  user_prompt     text,
  assistant_text  text,
  thinking        text,
  model           text,
  input_tokens    integer,
  output_tokens   integer,
  UNIQUE (session_id, ordinal)
);
CREATE INDEX turns_session ON turns (session_id, ordinal);
