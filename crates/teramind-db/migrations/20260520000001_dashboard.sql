-- Dashboard: persistent event log + benchmark history.

CREATE TABLE team_event_log (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind        text NOT NULL,
  user_id     uuid REFERENCES users(id),
  cwd         text,
  payload     jsonb NOT NULL,
  ts          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX team_event_log_recent      ON team_event_log (ts DESC);
CREATE INDEX team_event_log_user_recent ON team_event_log (user_id, ts DESC);
CREATE INDEX team_event_log_kind_recent ON team_event_log (kind, ts DESC);

CREATE TABLE quality_runs (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  baseline_label  text NOT NULL,
  model           text,
  ndcg10          double precision NOT NULL,
  mrr             double precision NOT NULL,
  precision_5     double precision NOT NULL,
  precision_10    double precision NOT NULL,
  recall_10       double precision NOT NULL,
  p50_latency_ms  double precision NOT NULL,
  p95_latency_ms  double precision NOT NULL,
  query_count     integer NOT NULL,
  corpus_size     integer NOT NULL,
  per_class       jsonb NOT NULL,
  raw_json        jsonb NOT NULL,
  ran_at          timestamptz NOT NULL DEFAULT now(),
  source          text NOT NULL CHECK (source IN ('scheduled','manual','ci'))
);
CREATE INDEX quality_runs_recent   ON quality_runs (ran_at DESC);
CREATE INDEX quality_runs_baseline ON quality_runs (baseline_label, ran_at DESC);
