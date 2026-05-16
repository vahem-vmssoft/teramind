CREATE TABLE wiki_pages (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id      uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  model           text NOT NULL,
  content         text NOT NULL,
  input_tokens    integer NOT NULL,
  output_tokens   integer NOT NULL,
  generated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (session_id, model)
);

CREATE INDEX wiki_pages_session ON wiki_pages (session_id);
CREATE INDEX wiki_pages_model   ON wiki_pages (model);
CREATE INDEX wiki_pages_recent  ON wiki_pages (generated_at DESC);

CREATE VIEW sessions_to_summarize AS
SELECT s.id AS session_id, s.cwd, s.started_at, s.ended_at, s.end_reason
FROM   sessions s
WHERE  s.ended_at IS NOT NULL;

DROP MATERIALIZED VIEW IF EXISTS traces_fts;

CREATE MATERIALIZED VIEW traces_fts AS
SELECT t.id            AS turn_id,
       t.session_id    AS session_id,
       t.ordinal       AS ordinal,
       t.started_at    AS ts,
       to_tsvector('english',
           coalesce(t.user_prompt, '')    || ' ' ||
           coalesce(t.assistant_text, '') || ' ' ||
           coalesce(t.thinking, '')       || ' ' ||
           coalesce(tc.output_agg, '')    || ' ' ||
           coalesce(fd.diff_agg, '')      || ' ' ||
           coalesce(wp.content, '')
       ) AS document
FROM turns t
LEFT JOIN LATERAL (
    SELECT string_agg(DISTINCT output, ' ') AS output_agg
    FROM tool_calls WHERE turn_id = t.id
) tc ON true
LEFT JOIN LATERAL (
    SELECT string_agg(DISTINCT unified_diff, ' ') AS diff_agg
    FROM file_diffs WHERE turn_id = t.id
) fd ON true
LEFT JOIN LATERAL (
    SELECT content FROM wiki_pages
    WHERE session_id = t.session_id
    ORDER BY generated_at DESC LIMIT 1
) wp ON true;

CREATE INDEX traces_fts_document     ON traces_fts USING gin (document);
CREATE UNIQUE INDEX traces_fts_turn_id ON traces_fts (turn_id);
