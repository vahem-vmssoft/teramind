CREATE MATERIALIZED VIEW traces_fts AS
SELECT
  t.id            AS turn_id,
  t.session_id    AS session_id,
  t.ordinal       AS ordinal,
  t.started_at    AS ts,
  to_tsvector('english',
      coalesce(t.user_prompt,'')   || ' ' ||
      coalesce(t.assistant_text,'') || ' ' ||
      coalesce(t.thinking,'')      || ' ' ||
      coalesce(string_agg(tc.output,' '),'')) AS document
FROM turns t
LEFT JOIN tool_calls tc ON tc.turn_id = t.id
GROUP BY t.id;

CREATE INDEX traces_fts_document ON traces_fts USING gin (document);
CREATE UNIQUE INDEX traces_fts_turn_id ON traces_fts (turn_id);
