CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE embeddings (
  id           bigserial PRIMARY KEY,
  item_kind    text NOT NULL CHECK (item_kind IN ('turn', 'file_diff')),
  item_id      uuid NOT NULL,
  model        text NOT NULL,
  dim          integer NOT NULL,
  embedding    vector(768) NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (item_kind, item_id, model)
);

CREATE INDEX embeddings_lookup ON embeddings (item_kind, item_id);
CREATE INDEX embeddings_model  ON embeddings (model);

CREATE INDEX embeddings_hnsw ON embeddings
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 16, ef_construction = 64);

CREATE VIEW traces_to_embed AS
SELECT 'turn'      AS kind,
       t.id        AS item_id,
       COALESCE(t.user_prompt, '') || ' ' || COALESCE(t.assistant_text, '') AS text
FROM   turns t
UNION ALL
SELECT 'file_diff' AS kind,
       d.id        AS item_id,
       d.pre_excerpt || ' ' || d.post_excerpt AS text
FROM   file_diffs d;
