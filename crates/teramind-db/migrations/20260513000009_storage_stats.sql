CREATE TABLE storage_stats (
  id            bigserial PRIMARY KEY,
  sampled_at    timestamptz NOT NULL DEFAULT now(),
  pg_bytes      bigint NOT NULL,
  jsonl_bytes   bigint NOT NULL,
  session_count bigint NOT NULL,
  turn_count    bigint NOT NULL,
  diff_count    bigint NOT NULL
);
CREATE INDEX storage_stats_sampled ON storage_stats (sampled_at DESC);
