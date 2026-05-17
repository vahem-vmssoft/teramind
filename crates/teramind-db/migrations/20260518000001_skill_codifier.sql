-- Skill codifier: detector output + candidate staging + cwd scope on skills.

CREATE TABLE skill_observations (
  id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind           text NOT NULL CHECK (kind IN ('tool_chain','problem_fix','llm_proposal')),
  signature      text NOT NULL,
  session_ids    uuid[] NOT NULL,
  frequency      integer NOT NULL,
  context_blob   jsonb NOT NULL,
  first_seen_at  timestamptz NOT NULL DEFAULT now(),
  last_seen_at   timestamptz NOT NULL DEFAULT now(),
  status         text NOT NULL DEFAULT 'open'
                   CHECK (status IN ('open','synthesized','skipped'))
);
CREATE UNIQUE INDEX skill_observations_sig ON skill_observations (kind, signature);
CREATE INDEX skill_observations_open_recent
  ON skill_observations (last_seen_at DESC) WHERE status = 'open';

CREATE TABLE skill_candidates (
  id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  observation_id      uuid NOT NULL REFERENCES skill_observations(id) ON DELETE CASCADE,
  name                text NOT NULL,
  description         text NOT NULL,
  body                text NOT NULL,
  applies_to_cwds     text[] NOT NULL,
  source_session_ids  uuid[] NOT NULL,
  model               text NOT NULL,
  input_tokens        integer NOT NULL,
  output_tokens       integer NOT NULL,
  generated_at        timestamptz NOT NULL DEFAULT now(),
  status              text NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','approved','rejected','promoted','superseded')),
  reviewer            text,
  reviewed_at         timestamptz
);
CREATE INDEX skill_candidates_pending
  ON skill_candidates (generated_at DESC) WHERE status = 'pending';
CREATE INDEX skill_candidates_obs ON skill_candidates (observation_id);
CREATE UNIQUE INDEX skill_candidates_open_name
  ON skill_candidates (name) WHERE status = 'pending';

ALTER TABLE skills ADD COLUMN applies_to_cwds text[] NOT NULL DEFAULT '{}';
CREATE INDEX skills_codified ON skills (updated_at DESC) WHERE source = 'codified';
