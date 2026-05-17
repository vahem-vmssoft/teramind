# Teramind Skill Codifier (Plan M) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wake up the `codified` skill source that has been reserved in the schema since Plan A. Mine repeated patterns out of captured sessions into reviewable skill candidates; admin-approved candidates promote into the live `skills` table and surface to Claude at SessionStart.

**Architecture:** Two-stage mining pipeline. Stage one: three pure-Rust detectors (tool_chain, problem_fix, llm_proposal) scan recent traces and UPSERT typed observations into a new `skill_observations` table. Stage two: a `codifier_worker` pulls above-threshold observations, bundles context with `Redactor::apply`, calls a `CodifyProvider` (Ollama default, Anthropic gated), and writes `skill_candidates` rows. Admin SQL `UPDATE status='approved'` triggers automatic promotion into `skills` on the next worker tick. SessionStart's `do_auto_recall` digest gains a "Relevant codified skills" section filtered by `applies_to_cwds` overlap with the current cwd ancestry.

**Tech Stack:** Rust 1.93 (workspace pin). No new workspace deps — reuses `sqlx`, `reqwest`, `serde_json`, `sha2`, `async-trait`, `parking_lot`. Reuses Plan H's `SummaryProvider` factory shape, Plan A's `Redactor::apply`, Plan F's `do_auto_recall` extension point, Plan K's `RpcTransport`, Plan J's `DecisionCache`.

---

## Spec coverage

This plan implements `docs/superpowers/specs/2026-05-17-teramind-skill-codifier-design.md` end-to-end. Coverage matrix at the bottom.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramind-db/migrations/20260518000001_skill_codifier.sql` | Tables + ALTER on skills |
| `crates/teramind-db/src/repos/skill_observation.rs` | `SkillObservationRepo` |
| `crates/teramind-db/src/repos/skill_candidate.rs` | `SkillCandidateRepo` |
| `crates/teramind-core/src/codify.rs` | `CodifyProvider` trait + `CodifyRequest` / `CodifyResult` / `CodifyDecision` |
| `crates/teramindd/src/config.rs` (extend) | `CodifyConfig` TOML loader |
| `crates/teramindd/src/services/codify/mod.rs` | Factory + re-exports |
| `crates/teramindd/src/services/codify/null.rs` | `NullCodifyProvider` (testing) |
| `crates/teramindd/src/services/codify/ollama.rs` | `OllamaCodifyProvider` |
| `crates/teramindd/src/services/codify/anthropic.rs` | `AnthropicCodifyProvider` (gated) |
| `crates/teramindd/src/services/codify/prompts.rs` | `SYSTEM_PROMPT` + snapshot |
| `crates/teramindd/src/services/codify/heuristics.rs` | Error regexes, signature normalizers, diff_kind classifier |
| `crates/teramindd/src/services/codify/glob.rs` | `applies_to_cwds` glob matcher |
| `crates/teramindd/src/services/codify/detectors/mod.rs` | Detector module registry + `Observation` type |
| `crates/teramindd/src/services/codify/detectors/tool_chain.rs` | Detector A |
| `crates/teramindd/src/services/codify/detectors/problem_fix.rs` | Detector B |
| `crates/teramindd/src/services/codify/detectors/llm_proposal.rs` | Detector C |
| `crates/teramindd/src/services/codify/synthesis.rs` | Bundler + provider call + parse |
| `crates/teramindd/src/services/codify/promote.rs` | Transactional candidate → skill promotion |
| `crates/teramindd/src/services/codifier_worker.rs` | 3-loop spawn + `CodifierStats` |
| `crates/teramind/src/commands/skills.rs` | `teramind skills list/show/observations` |
| `crates/teramindd/tests/codify_e2e.rs` | End-to-end synthesis + promote |
| `crates/teramindd/tests/codify_privacy.rs` | DecisionCache filter |
| `crates/teramind/tests/skills_cli.rs` | CLI surface smoke |

**Modified files:**

- `crates/teramind-core/src/lib.rs` — `pub mod codify;`
- `crates/teramind-db/src/repos/mod.rs` — register `skill_observation` + `skill_candidate`
- `crates/teramind-db/src/repos/skill.rs` — read/write `applies_to_cwds`
- `crates/teramind-ipc/src/proto.rs` — `Request::{CodifyNow, SkillsList, SkillsShow, SkillsObservations}`
- `crates/teramindd/src/services/rpc_dispatch.rs` — handle the new variants
- `crates/teramindd/src/services/ipc_server.rs` — combined match arm for new variants
- `crates/teramindd/src/services/mod.rs` — register `codify` + `codifier_worker`
- `crates/teramindd/src/services/search.rs::do_auto_recall` — append "Relevant codified skills" section
- `crates/teramindd/src/app.rs` — load `CodifyConfig`, spawn `codifier_worker`
- `crates/teramind-mcp/src/server.rs` — `mcp__teramind__codify` tool
- `crates/teramind/src/cli.rs` — `Skills { action }` variant
- `crates/teramind/src/commands/mod.rs` — register `skills`
- `crates/teramind/src/commands/doctor.rs` — codifier health lines

---

## Section 0 — Pre-flight

### Task 0.1: Branch from a green main

- [ ] **Step 1**

Run:
```bash
git checkout main
cargo build --workspace
git checkout -b feat/teramind-skill-codifier
git log --oneline -3
```

Expect: build silent. HEAD on new branch. Recent commits include Plan L merge + spec commit `7400e5b`.

### Task 0.2: Confirm GitHub token for PG tests

- [ ] **Step 1**

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramind-db --test user_repo -- --test-threads=1 | tail -3`

Expected: 2 PASS. (See `memory/teramind_pg_supervisor_pgvector.md` — `GITHUB_TOKEN` is required for the embedded PG fixture to download pgvector.)

---

## Section 1 — Migration + new tables

### Task 1.1: Write the SQL migration

**Files:** Create `crates/teramind-db/migrations/20260518000001_skill_codifier.sql`.

- [ ] **Step 1: Write the migration**

```sql
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
```

- [ ] **Step 2: Write the migration test**

**File:** Create `crates/teramind-db/tests/skill_codifier_migration.rs`.

```rust
//! Verifies the skill-codifier migration applies cleanly.

use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn migration_creates_observation_and_candidate_tables() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    for t in ["skill_observations", "skill_candidates"] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)"
        ).bind(t).fetch_one(pool.pg()).await?;
        assert!(exists, "table `{t}` must exist after migration");
    }

    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'skills' AND column_name = 'applies_to_cwds')"
    ).fetch_one(pool.pg()).await?;
    assert!(exists, "skills.applies_to_cwds must exist after migration");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 3: Run**

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramind-db --test skill_codifier_migration -- --test-threads=1`

Expected: 1 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/migrations/20260518000001_skill_codifier.sql \
        crates/teramind-db/tests/skill_codifier_migration.rs
git commit -m "feat(db): skill codifier migration (observations + candidates + applies_to_cwds)"
```

---

## Section 2 — `SkillObservationRepo`

### Task 2.1: Failing test

**File:** Create `crates/teramind-db/tests/skill_observation_repo.rs`.

- [ ] **Step 1: Write the tests**

```rust
use teramind_db::repos::SkillObservationRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_core::ids::SessionId;
use uuid::Uuid;

async fn pool_with_migrations() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upsert_merges_session_ids_and_bumps_frequency() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let r = SkillObservationRepo::new(pool.clone());

    let sa = SessionId(Uuid::new_v4());
    let sb = SessionId(Uuid::new_v4());

    r.upsert("tool_chain", "sigA", &[sa], serde_json::json!({"k":"v"})).await?;
    let obs1 = r.find_by_sig("tool_chain", "sigA").await?.unwrap();
    assert_eq!(obs1.frequency, 1);

    r.upsert("tool_chain", "sigA", &[sb], serde_json::json!({"k":"v"})).await?;
    let obs2 = r.find_by_sig("tool_chain", "sigA").await?.unwrap();
    assert_eq!(obs2.frequency, 2);
    assert!(obs2.session_ids.contains(&sa.0) && obs2.session_ids.contains(&sb.0));

    // Same session twice does not double-count.
    r.upsert("tool_chain", "sigA", &[sa], serde_json::json!({"k":"v"})).await?;
    let obs3 = r.find_by_sig("tool_chain", "sigA").await?.unwrap();
    assert_eq!(obs3.frequency, 2, "duplicate session must not increment frequency");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_open_filters_by_threshold() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let r = SkillObservationRepo::new(pool.clone());

    for i in 0..3 {
        r.upsert("tool_chain", &format!("sig{i}"),
                 &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    }
    // One observation gets 3 sessions.
    let sigs_high = "sig0";
    r.upsert("tool_chain", sigs_high, &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    r.upsert("tool_chain", sigs_high, &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;

    let above = r.list_open(3, 10).await?;
    assert_eq!(above.len(), 1);
    assert_eq!(above[0].signature, "sig0");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mark_status_transitions() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let r = SkillObservationRepo::new(pool.clone());

    r.upsert("tool_chain", "sigX", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let obs = r.find_by_sig("tool_chain", "sigX").await?.unwrap();
    r.mark_synthesized(obs.id).await?;
    let after = r.find_by_sig("tool_chain", "sigX").await?.unwrap();
    assert_eq!(after.status, "synthesized");

    sup.shutdown().await?;
    Ok(())
}
```

Run: `cargo test -p teramind-db --test skill_observation_repo -- --test-threads=1` → FAIL (`SkillObservationRepo` missing).

### Task 2.2: Implement

**File:** Create `crates/teramind-db/src/repos/skill_observation.rs`.

- [ ] **Step 1: Add `SkillObservationId` newtype**

In `crates/teramind-core/src/ids.rs`, add: `id_newtype!(SkillObservationId);`

- [ ] **Step 2: Write the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use serde_json::Value;
use teramind_core::ids::{SessionId, SkillObservationId};
use time::OffsetDateTime;
use uuid::Uuid;

type ObservationRow = (Uuid, String, String, Vec<Uuid>, i32, Value, OffsetDateTime, OffsetDateTime, String);

fn row_to_observation(r: ObservationRow) -> Observation {
    Observation {
        id: SkillObservationId(r.0),
        kind: r.1,
        signature: r.2,
        session_ids: r.3,
        frequency: r.4,
        context_blob: r.5,
        first_seen_at: r.6,
        last_seen_at: r.7,
        status: r.8,
    }
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub id: SkillObservationId,
    pub kind: String,
    pub signature: String,
    pub session_ids: Vec<Uuid>,
    pub frequency: i32,
    pub context_blob: Value,
    pub first_seen_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub status: String,
}

#[derive(Clone)]
pub struct SkillObservationRepo { pool: DbPool }

impl SkillObservationRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    /// UPSERT keyed by (kind, signature). Appends new session_ids and bumps frequency.
    pub async fn upsert(
        &self,
        kind: &str,
        signature: &str,
        new_sessions: &[SessionId],
        context_blob: Value,
    ) -> Result<()> {
        let new_uuids: Vec<Uuid> = new_sessions.iter().map(|s| s.0).collect();
        sqlx::query(
            r#"
            INSERT INTO skill_observations (kind, signature, session_ids, frequency, context_blob)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (kind, signature) DO UPDATE
              SET session_ids = (
                    SELECT ARRAY(SELECT DISTINCT unnest(skill_observations.session_ids || EXCLUDED.session_ids))
                  ),
                  frequency = (
                    SELECT cardinality(ARRAY(SELECT DISTINCT unnest(skill_observations.session_ids || EXCLUDED.session_ids)))
                  ),
                  last_seen_at = now(),
                  context_blob = EXCLUDED.context_blob
            "#)
            .bind(kind).bind(signature).bind(&new_uuids).bind(new_uuids.len() as i32)
            .bind(context_blob)
            .execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn find_by_sig(&self, kind: &str, signature: &str) -> Result<Option<Observation>> {
        let row: Option<ObservationRow> = sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations WHERE kind = $1 AND signature = $2"#)
            .bind(kind).bind(signature)
            .fetch_optional(self.pool.pg()).await?;
        Ok(row.map(row_to_observation))
    }

    pub async fn list_open(&self, min_frequency: i32, limit: i64) -> Result<Vec<Observation>> {
        let rows: Vec<ObservationRow> = sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations
               WHERE status = 'open' AND frequency >= $1
               ORDER BY last_seen_at ASC
               LIMIT $2"#)
            .bind(min_frequency).bind(limit)
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_observation).collect())
    }

    pub async fn list_recent(&self, kind: Option<&str>, status: Option<&str>, limit: i64) -> Result<Vec<Observation>> {
        let rows: Vec<ObservationRow> = sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations
               WHERE ($1::text IS NULL OR kind = $1)
                 AND ($2::text IS NULL OR status = $2)
               ORDER BY last_seen_at DESC
               LIMIT $3"#)
            .bind(kind).bind(status).bind(limit)
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_observation).collect())
    }

    pub async fn mark_synthesized(&self, id: SkillObservationId) -> Result<()> {
        sqlx::query("UPDATE skill_observations SET status='synthesized' WHERE id=$1")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn mark_skipped(&self, id: SkillObservationId, reason: &str) -> Result<()> {
        sqlx::query(
            r#"UPDATE skill_observations
               SET status='skipped',
                   context_blob = jsonb_set(context_blob, '{skip_reason}', to_jsonb($2::text))
               WHERE id=$1"#)
            .bind(id.0).bind(reason).execute(self.pool.pg()).await?;
        Ok(())
    }
}
```

- [ ] **Step 3: Register**

In `crates/teramind-db/src/repos/mod.rs`, add `pub mod skill_observation;` and `pub use skill_observation::{Observation, SkillObservationRepo};`.

- [ ] **Step 4: Run**

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramind-db --test skill_observation_repo -- --test-threads=1`

Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/ids.rs \
        crates/teramind-db/src/repos/skill_observation.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/tests/skill_observation_repo.rs
git commit -m "feat(db): SkillObservationRepo"
```

---

## Section 3 — `SkillCandidateRepo` + applies_to_cwds on Skill

### Task 3.1: Failing test

**File:** Create `crates/teramind-db/tests/skill_candidate_repo.rs`.

- [ ] **Step 1: Write tests**

```rust
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_core::ids::SessionId;
use uuid::Uuid;

async fn pool_with_migrations() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_then_list_pending() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let obs = SkillObservationRepo::new(pool.clone());
    let cand = SkillCandidateRepo::new(pool.clone());

    obs.upsert("tool_chain", "sig1", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let o = obs.find_by_sig("tool_chain", "sig1").await?.unwrap();

    cand.insert(
        o.id, "rust-pr-prep", "Build + test + commit", "# rust-pr-prep\n…",
        &["/openvms-*".into()],
        &[SessionId(Uuid::new_v4())],
        "ollama:qwen3.6:latest", 1200, 350,
    ).await?;

    let pending = cand.list_pending(10).await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "rust-pr-prep");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approve_then_list_approved() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let obs = SkillObservationRepo::new(pool.clone());
    let cand = SkillCandidateRepo::new(pool.clone());

    obs.upsert("tool_chain", "sig1", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let o = obs.find_by_sig("tool_chain", "sig1").await?.unwrap();
    cand.insert(o.id, "n", "d", "b", &[], &[], "m", 0, 0).await?;
    let pending = cand.list_pending(10).await?;
    let id = pending[0].id;

    // Approval is just SQL UPDATE per spec §3.
    sqlx::query("UPDATE skill_candidates SET status='approved', reviewer='admin', reviewed_at=now() WHERE id=$1")
        .bind(id.0).execute(pool.pg()).await?;

    let approved = cand.list_approved(10).await?;
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].id, id);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unique_pending_name_constraint() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let obs = SkillObservationRepo::new(pool.clone());
    let cand = SkillCandidateRepo::new(pool.clone());

    obs.upsert("tool_chain", "sigA", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let oa = obs.find_by_sig("tool_chain", "sigA").await?.unwrap();
    cand.insert(oa.id, "dup-name", "d", "b", &[], &[], "m", 0, 0).await?;

    obs.upsert("tool_chain", "sigB", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let ob = obs.find_by_sig("tool_chain", "sigB").await?.unwrap();
    let res = cand.insert(ob.id, "dup-name", "d", "b", &[], &[], "m", 0, 0).await;
    assert!(res.is_err(), "second pending with same name must fail unique constraint");

    sup.shutdown().await?;
    Ok(())
}
```

Run: `cargo test -p teramind-db --test skill_candidate_repo` → FAIL.

### Task 3.2: Implement

**File:** Create `crates/teramind-db/src/repos/skill_candidate.rs`.

- [ ] **Step 1: Add SkillCandidateId**

In `crates/teramind-core/src/ids.rs`: `id_newtype!(SkillCandidateId);`

- [ ] **Step 2: Write the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{SessionId, SkillCandidateId, SkillObservationId};
use time::OffsetDateTime;
use uuid::Uuid;

type CandidateRow = (
    Uuid, Uuid, String, String, String,
    Vec<String>, Vec<Uuid>, String, i32, i32,
    OffsetDateTime, String, Option<String>, Option<OffsetDateTime>,
);

fn row_to_candidate(r: CandidateRow) -> Candidate {
    Candidate {
        id: SkillCandidateId(r.0),
        observation_id: SkillObservationId(r.1),
        name: r.2, description: r.3, body: r.4,
        applies_to_cwds: r.5,
        source_session_ids: r.6,
        model: r.7,
        input_tokens: r.8, output_tokens: r.9,
        generated_at: r.10,
        status: r.11,
        reviewer: r.12,
        reviewed_at: r.13,
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: SkillCandidateId,
    pub observation_id: SkillObservationId,
    pub name: String,
    pub description: String,
    pub body: String,
    pub applies_to_cwds: Vec<String>,
    pub source_session_ids: Vec<Uuid>,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub generated_at: OffsetDateTime,
    pub status: String,
    pub reviewer: Option<String>,
    pub reviewed_at: Option<OffsetDateTime>,
}

#[derive(Clone)]
pub struct SkillCandidateRepo { pool: DbPool }

impl SkillCandidateRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert(
        &self,
        observation_id: SkillObservationId,
        name: &str,
        description: &str,
        body: &str,
        applies_to_cwds: &[String],
        source_session_ids: &[SessionId],
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
    ) -> Result<SkillCandidateId> {
        let sids: Vec<Uuid> = source_session_ids.iter().map(|s| s.0).collect();
        let row: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO skill_candidates
               (observation_id, name, description, body, applies_to_cwds,
                source_session_ids, model, input_tokens, output_tokens)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               RETURNING id"#)
            .bind(observation_id.0)
            .bind(name).bind(description).bind(body)
            .bind(applies_to_cwds).bind(&sids)
            .bind(model).bind(input_tokens).bind(output_tokens)
            .fetch_one(self.pool.pg()).await?;
        Ok(SkillCandidateId(row.0))
    }

    pub async fn list_pending(&self, limit: i64) -> Result<Vec<Candidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates WHERE status='pending'
               ORDER BY generated_at DESC
               LIMIT $1"#)
            .bind(limit).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_candidate).collect())
    }

    pub async fn list_approved(&self, limit: i64) -> Result<Vec<Candidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates WHERE status='approved'
               ORDER BY reviewed_at ASC
               LIMIT $1
               FOR UPDATE SKIP LOCKED"#)
            .bind(limit).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_candidate).collect())
    }

    pub async fn list_filter(&self, status: Option<&str>, limit: i64) -> Result<Vec<Candidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates
               WHERE ($1::text IS NULL OR status = $1)
               ORDER BY generated_at DESC
               LIMIT $2"#)
            .bind(status).bind(limit)
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_candidate).collect())
    }

    pub async fn mark_promoted(&self, id: SkillCandidateId) -> Result<()> {
        sqlx::query("UPDATE skill_candidates SET status='promoted' WHERE id=$1")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    /// Returns the previous candidate ids that were marked `superseded`
    /// (a candidate from the same observation that was still `pending`).
    pub async fn supersede_prior(&self, observation_id: SkillObservationId, exclude_id: SkillCandidateId) -> Result<u64> {
        let r = sqlx::query(
            r#"UPDATE skill_candidates
               SET status='superseded'
               WHERE observation_id = $1 AND id != $2 AND status='pending'"#)
            .bind(observation_id.0).bind(exclude_id.0)
            .execute(self.pool.pg()).await?;
        Ok(r.rows_affected())
    }
}
```

- [ ] **Step 3: Register**

In `crates/teramind-db/src/repos/mod.rs`: `pub mod skill_candidate;` + `pub use skill_candidate::{Candidate, SkillCandidateRepo};`.

- [ ] **Step 4: Extend `SkillRepo` to read/write `applies_to_cwds`**

Modify `crates/teramind-db/src/repos/skill.rs`. Add a richer upsert that includes the new column:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn upsert_codified(
    &self,
    name: &str,
    description: &str,
    body: &str,
    source_session_ids: &[uuid::Uuid],
    applies_to_cwds: &[String],
) -> Result<SkillId> {
    let r: (uuid::Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO skills (name, description, body, source, source_session_ids, applies_to_cwds)
        VALUES ($1,$2,$3,'codified',$4,$5)
        ON CONFLICT (name) DO UPDATE SET
            description=EXCLUDED.description,
            body=EXCLUDED.body,
            source_session_ids=EXCLUDED.source_session_ids,
            applies_to_cwds=EXCLUDED.applies_to_cwds,
            updated_at=now()
        RETURNING id
        "#)
        .bind(name).bind(description).bind(body)
        .bind(source_session_ids).bind(applies_to_cwds)
        .fetch_one(self.pool.pg()).await?;
    Ok(SkillId(r.0))
}

/// List all codified skills whose applies_to_cwds match (or are empty/global).
pub async fn list_codified_for_cwd(&self, cwd: &str, limit: i64) -> Result<Vec<(SkillId, String, String, Vec<String>, i32)>> {
    // The glob match is done in Rust (cheap and we already have the matcher);
    // here we just fetch everything codified.
    let rows: Vec<(uuid::Uuid, String, String, Vec<String>, Vec<uuid::Uuid>)> = sqlx::query_as(
        r#"SELECT id, name, description, applies_to_cwds, source_session_ids
           FROM skills WHERE source = 'codified'
           ORDER BY updated_at DESC
           LIMIT $1"#)
        .bind(limit).fetch_all(self.pool.pg()).await?;
    // Filter in Rust: this fn returns (id, name, description, applies_to_cwds, seeded_from_count).
    // Caller decides the glob match.
    let _ = cwd; // matching happens in caller
    Ok(rows.into_iter()
        .map(|r| (SkillId(r.0), r.1, r.2, r.3, r.4.len() as i32))
        .collect())
}
```

- [ ] **Step 5: Run**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-db --test skill_candidate_repo -- --test-threads=1
cargo clippy -p teramind-db --all-targets -- -D warnings
```

Expected: 3 PASS, clippy silent.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-core/src/ids.rs \
        crates/teramind-db/src/repos/skill_candidate.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/src/repos/skill.rs \
        crates/teramind-db/tests/skill_candidate_repo.rs
git commit -m "feat(db): SkillCandidateRepo + applies_to_cwds on SkillRepo"
```

---

## Section 4 — `CodifyProvider` trait in `teramind-core`

### Task 4.1: Define the trait

**Files:**
- Create: `crates/teramind-core/src/codify.rs`
- Modify: `crates/teramind-core/src/lib.rs`

- [ ] **Step 1: Write the module**

```rust
//! Skill-codifier provider trait. Pure data + trait; impls live under
//! teramindd::services::codify::{ollama, anthropic, null}.

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct CodifyRequest {
    pub observation_kind: String,
    pub bundled_context: String,
    pub frequency: u32,
    pub cwds: Vec<String>,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct CodifyResult {
    pub decision: CodifyDecision,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone)]
pub enum CodifyDecision {
    Skip {
        reason: String,
    },
    Skill {
        name: String,
        description: String,
        body: String,
        applies_to_cwds: Vec<String>,
    },
}

#[async_trait]
pub trait CodifyProvider: Send + Sync {
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult>;
    fn name(&self) -> &str;
}
```

- [ ] **Step 2: Register**

In `crates/teramind-core/src/lib.rs`, add `pub mod codify;` alphabetically.

- [ ] **Step 3: Verify**

Run: `cargo build -p teramind-core && cargo clippy -p teramind-core --all-targets -- -D warnings`

Expected: silent.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/codify.rs crates/teramind-core/src/lib.rs
git commit -m "feat(core): CodifyProvider trait"
```

---

## Section 5 — `applies_to_cwds` glob matcher

### Task 5.1: Pure-logic module + tests

**File:** Create `crates/teramindd/src/services/codify/glob.rs`.

- [ ] **Step 1: Write the module**

```rust
//! Minimal cwd glob matcher. Supports:
//! - Plain prefix:        `/Users/alice/proj`   matches `/Users/alice/proj/sub/file`
//! - `*` segment wildcard: `/openvms-*`        matches `/openvms-rsync`, `/openvms-llvm`
//! - Empty pattern list = global (matches all).
//!
//! We do NOT use a full glob crate — the language is intentionally tiny so
//! the SessionStart digest filter is O(N skills × M patterns) with cheap
//! per-comparison work.

pub fn matches(pattern: &str, cwd: &str) -> bool {
    if pattern.is_empty() { return false; }
    // Plain prefix (no `*`): treat as ancestor match.
    if !pattern.contains('*') {
        return cwd == pattern || cwd.starts_with(&format!("{pattern}/"));
    }
    // Segment-wildcard match.
    let pat_segs: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let cwd_segs: Vec<&str> = cwd.trim_start_matches('/').split('/').collect();
    if pat_segs.len() > cwd_segs.len() { return false; }
    for (p, c) in pat_segs.iter().zip(cwd_segs.iter()) {
        if !segment_matches(p, c) { return false; }
    }
    true
}

pub fn matches_any(patterns: &[String], cwd: &str) -> bool {
    if patterns.is_empty() { return true; } // global
    patterns.iter().any(|p| matches(p, cwd))
}

fn segment_matches(pat: &str, seg: &str) -> bool {
    if pat == "*" { return true; }
    if !pat.contains('*') { return pat == seg; }
    // Simple two-side wildcard: `prefix*suffix`. Only one `*` supported.
    let parts: Vec<&str> = pat.splitn(2, '*').collect();
    let (pre, post) = (parts[0], parts[1]);
    seg.starts_with(pre) && seg.ends_with(post) && seg.len() >= pre.len() + post.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_prefix_matches_self_and_descendants() {
        assert!(matches("/Users/alice/proj", "/Users/alice/proj"));
        assert!(matches("/Users/alice/proj", "/Users/alice/proj/sub"));
        assert!(!matches("/Users/alice/proj", "/Users/alice/other"));
        assert!(!matches("/Users/alice/proj", "/Users/alice/projection"));
    }

    #[test]
    fn segment_wildcard_matches() {
        assert!(matches("/openvms-*", "/openvms-rsync"));
        assert!(matches("/openvms-*", "/openvms-llvm"));
        assert!(matches("/openvms-*", "/openvms-rsync/src"));
        assert!(!matches("/openvms-*", "/openssl-vms"));
    }

    #[test]
    fn empty_pattern_does_not_match() {
        assert!(!matches("", "/anything"));
    }

    #[test]
    fn matches_any_empty_is_global() {
        assert!(matches_any(&[], "/anywhere"));
    }

    #[test]
    fn matches_any_with_patterns() {
        let ps = vec!["/openvms-*".to_string(), "/Users/alice/proj".to_string()];
        assert!(matches_any(&ps, "/openvms-rsync"));
        assert!(matches_any(&ps, "/Users/alice/proj/sub"));
        assert!(!matches_any(&ps, "/some/other/path"));
    }
}
```

- [ ] **Step 2: Stub the codify module tree**

Create `crates/teramindd/src/services/codify/mod.rs`:

```rust
//! Skill codifier subsystem.

pub mod glob;
```

In `crates/teramindd/src/services/mod.rs`, add `pub mod codify;`.

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd services::codify::glob::`

Expected: 5 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/codify/mod.rs \
        crates/teramindd/src/services/codify/glob.rs \
        crates/teramindd/src/services/mod.rs
git commit -m "feat(codify): cwd glob matcher"
```

---

## Section 6 — Heuristics (error patterns + signature normalizers + diff_kind)

### Task 6.1: Implement + tests

**File:** Create `crates/teramindd/src/services/codify/heuristics.rs`.

- [ ] **Step 1: Write the module**

```rust
//! Shared heuristics for the codifier's pattern detectors.

use once_cell::sync::Lazy;
use regex::Regex;

/// Regex set indicating a turn is error-shaped (caller decides what counts).
static ERROR_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"(?m)^error:",
        r"(?m)^Error:",
        r"panicked at",
        r"^Traceback",
        r"FAILED",
        r"clippy::[a-z_]+",
        r"cannot find ",
        r"undefined reference",
    ].iter().map(|p| Regex::new(p).unwrap()).collect()
});

pub fn looks_like_error(text: &str) -> bool {
    ERROR_PATTERNS.iter().any(|r| r.is_match(text))
}

/// Normalize an error string for signature hashing:
/// - Strip line/column numbers (`:123:45`).
/// - Replace generic identifiers `\w+` with `<id>` (but keep keywords like `error`, `Traceback`).
/// - Truncate to 80 chars.
pub fn normalize_error(text: &str) -> String {
    let re_line = Regex::new(r":\d+(:\d+)?").unwrap();
    let re_ident = Regex::new(r"\b[a-zA-Z_][a-zA-Z0-9_]+\b").unwrap();
    let no_lines = re_line.replace_all(text, "");
    let keywords = ["error", "Error", "panicked", "Traceback", "FAILED", "cannot", "find", "undefined", "reference", "clippy"];
    let normalized = re_ident.replace_all(&no_lines, |caps: &regex::Captures| {
        let word = &caps[0];
        if keywords.iter().any(|k| word.eq_ignore_ascii_case(k)) {
            word.to_string()
        } else {
            "<id>".to_string()
        }
    });
    normalized.chars().take(80).collect()
}

/// Classify a unified diff string into a coarse kind. Heuristic, not AST-aware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    AddedBlock,
    RemovedBlock,
    SignatureChange,
    Rename,
    Mixed,
}

impl DiffKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DiffKind::AddedBlock     => "added_block",
            DiffKind::RemovedBlock   => "removed_block",
            DiffKind::SignatureChange=> "signature_change",
            DiffKind::Rename         => "rename",
            DiffKind::Mixed          => "mixed",
        }
    }
}

pub fn classify_diff(diff: &str) -> DiffKind {
    let adds = diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
    let dels = diff.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
    let renames = diff.lines().any(|l| l.starts_with("rename from ") || l.starts_with("similarity index "));
    let sig_change = diff.lines().any(|l| (l.starts_with('+') || l.starts_with('-')) &&
        (l.contains("fn ") || l.contains("def ") || l.contains("function ")));

    if renames { return DiffKind::Rename; }
    if sig_change { return DiffKind::SignatureChange; }
    if adds > 0 && dels == 0 { return DiffKind::AddedBlock; }
    if dels > 0 && adds == 0 { return DiffKind::RemovedBlock; }
    DiffKind::Mixed
}

/// Head verb of a Bash command: first whitespace-separated token, lowercased.
pub fn bash_head_verb(cmd: &str) -> &str {
    cmd.split_whitespace().next().unwrap_or("").trim_start_matches("./")
}

/// Extension of a file path, or `_` if none.
pub fn file_kind(path: &str) -> String {
    match path.rsplit('.').next() {
        Some(ext) if ext != path && !ext.is_empty() => format!(".{ext}"),
        _ => "_".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_patterns_detect_common_shapes() {
        assert!(looks_like_error("error: expected `;`"));
        assert!(looks_like_error("thread 'main' panicked at foo.rs"));
        assert!(looks_like_error("Traceback (most recent call last):"));
        assert!(looks_like_error("FAILED: 3 tests"));
        assert!(!looks_like_error("everything is fine"));
    }

    #[test]
    fn normalize_strips_line_numbers_and_identifiers() {
        let a = normalize_error("error: cannot find `foo` at file.rs:42:10");
        let b = normalize_error("error: cannot find `bar` at other.rs:99:1");
        assert_eq!(a, b, "line numbers and ident names must collapse to the same form");
    }

    #[test]
    fn classify_diff_kinds() {
        assert_eq!(classify_diff("+ added line\n"), DiffKind::AddedBlock);
        assert_eq!(classify_diff("- removed line\n"), DiffKind::RemovedBlock);
        assert_eq!(classify_diff("- pub fn foo() {}\n+ pub fn foo(x: i32) {}\n"), DiffKind::SignatureChange);
        assert_eq!(classify_diff("rename from a.rs\n"), DiffKind::Rename);
        assert_eq!(classify_diff("+ a\n- b\n"), DiffKind::Mixed);
    }

    #[test]
    fn bash_head_verb_basic() {
        assert_eq!(bash_head_verb("cargo build --release"), "cargo");
        assert_eq!(bash_head_verb("./scripts/run.sh foo"), "scripts/run.sh");
    }

    #[test]
    fn file_kind_returns_dot_ext_or_underscore() {
        assert_eq!(file_kind("foo.rs"), ".rs");
        assert_eq!(file_kind("path/to/Cargo.toml"), ".toml");
        assert_eq!(file_kind("Makefile"), "_");
    }
}
```

- [ ] **Step 2: Add deps**

In `crates/teramindd/Cargo.toml`, ensure these are in `[dependencies]` (most are already there via Plans A–L; check and add only missing):
- `once_cell = { workspace = true }`
- `regex = { workspace = true }`

- [ ] **Step 3: Register**

In `crates/teramindd/src/services/codify/mod.rs`, add `pub mod heuristics;`.

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd services::codify::heuristics::`

Expected: 5 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/Cargo.toml \
        crates/teramindd/src/services/codify/heuristics.rs \
        crates/teramindd/src/services/codify/mod.rs
git commit -m "feat(codify): heuristics (error patterns + diff_kind + signature helpers)"
```

---

## Section 7 — Detector A: tool_chain

### Task 7.1: Failing test

**File:** Create `crates/teramindd/tests/detector_tool_chain.rs`.

- [ ] **Step 1**

```rust
//! Detector A: 5 sessions with identical Bash→Edit→Bash chains produce one
//! observation with frequency=5.

use teramind_core::ids::{AgentId, SessionId, TurnId};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::tool_chain;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_identical_chains_produce_one_observation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();

    for _ in 0..5 {
        let sid = sessions.insert(NewSession {
            agent_id: agent.id, agent_session_id: None, cwd: "/proj",
            project_id: None, parent_session_id: None,
            git_head: None, git_branch: None,
            os: "linux", hostname: "h", user_login: "u",
            started_at: started, user_id: None, device_id: None,
        }).await?;
        let tid = trace.upsert_turn_with_id(
            TurnId(Uuid::new_v4()), sid, 0, started,
            Some("build it"),
        ).await?;
        trace.finalize_turn(tid, started, Some("done"), None, Some("claude"), None, None).await?;
        // Three tool calls: cargo build, edit Cargo.toml, cargo test.
        trace.insert_tool_call(tid, 0, "Bash", &serde_json::json!({"command":"cargo build"}),
                               Some("ok"), false, 100, started).await?;
        trace.insert_tool_call(tid, 1, "Edit", &serde_json::json!({"file_path":"Cargo.toml"}),
                               Some("ok"), false, 50, started).await?;
        trace.insert_tool_call(tid, 2, "Bash", &serde_json::json!({"command":"cargo test"}),
                               Some("ok"), false, 100, started).await?;
    }

    let obs_repo = SkillObservationRepo::new(pool.clone());
    tool_chain::run(&pool, &obs_repo, time::Duration::days(30)).await?;

    let above = obs_repo.list_open(3, 10).await?;
    assert_eq!(above.len(), 1, "exactly one observation above threshold");
    assert_eq!(above[0].frequency, 5);
    assert_eq!(above[0].kind, "tool_chain");

    sup.shutdown().await?;
    Ok(())
}
```

(Adapt `TraceRepo::insert_tool_call` to the actual existing signature — inspect `crates/teramind-db/src/repos/trace.rs` before writing the test.)

Run: `cargo test -p teramindd --test detector_tool_chain -- --test-threads=1` → FAIL (no `tool_chain::run`).

### Task 7.2: Implement

**File:** Create `crates/teramindd/src/services/codify/detectors/mod.rs` and `tool_chain.rs`.

- [ ] **Step 1: Write detectors/mod.rs**

```rust
pub mod tool_chain;
pub mod problem_fix;
pub mod llm_proposal;
```

- [ ] **Step 2: Write detectors/tool_chain.rs**

```rust
//! Detector A — repeated tool-call sequences.

use crate::services::codify::heuristics::{bash_head_verb, file_kind};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use teramind_core::ids::SessionId;
use teramind_db::pool::DbPool;
use teramind_db::repos::SkillObservationRepo;

#[derive(Debug, Clone)]
struct CallRow {
    session_id: uuid::Uuid,
    tool_name: String,
    input: serde_json::Value,
    started_at: time::OffsetDateTime,
}

pub async fn run(
    pool: &DbPool,
    obs: &SkillObservationRepo,
    window: time::Duration,
) -> anyhow::Result<()> {
    let cutoff = time::OffsetDateTime::now_utc() - window;

    let rows: Vec<(uuid::Uuid, String, serde_json::Value, time::OffsetDateTime)> = sqlx::query_as(
        r#"SELECT t.session_id, tc.name, tc.input, tc.started_at
           FROM tool_calls tc
           JOIN turns t ON t.id = tc.turn_id
           JOIN sessions s ON s.id = t.session_id
           WHERE tc.started_at >= $1
           ORDER BY t.session_id, tc.started_at"#)
        .bind(cutoff)
        .fetch_all(pool.pg()).await?;

    let mut per_session: HashMap<uuid::Uuid, Vec<CallRow>> = HashMap::new();
    for (sid, name, input, ts) in rows {
        per_session.entry(sid).or_default().push(CallRow {
            session_id: sid, tool_name: name, input, started_at: ts,
        });
    }

    // Per session, build a signature tuple list.
    let mut sig_to_sessions: HashMap<String, Vec<SessionId>> = HashMap::new();
    let mut sig_to_chain: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (sid, calls) in per_session {
        let tuples: Vec<(String, String)> = calls.iter().map(|c| {
            let head = match c.tool_name.as_str() {
                "Bash" => bash_head_verb(c.input.get("command").and_then(|v| v.as_str()).unwrap_or("")).to_string(),
                "Edit" | "Write" | "Read" => file_kind(c.input.get("file_path").and_then(|v| v.as_str()).unwrap_or("")),
                _ => String::new(),
            };
            (c.tool_name.clone(), head)
        }).collect();
        if tuples.len() < 2 { continue; }
        let mut hasher = Sha256::new();
        for (t, h) in &tuples {
            hasher.update(t.as_bytes());
            hasher.update(b"\x00");
            hasher.update(h.as_bytes());
            hasher.update(b"\x01");
        }
        let sig = hex::encode(&hasher.finalize()[..8]);
        sig_to_sessions.entry(sig.clone()).or_default().push(SessionId(sid));
        sig_to_chain.entry(sig).or_insert(tuples);
    }

    for (sig, sessions) in sig_to_sessions {
        if sessions.is_empty() { continue; }
        let chain = sig_to_chain.get(&sig).cloned().unwrap_or_default();
        let context = serde_json::json!({
            "head_chain": chain.iter().map(|(t, h)| format!("{t}({h})")).collect::<Vec<_>>(),
        });
        obs.upsert("tool_chain", &sig, &sessions, context).await?;
    }
    Ok(())
}
```

- [ ] **Step 3: Run**

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramindd --test detector_tool_chain -- --test-threads=1`

Expected: 1 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/codify/detectors \
        crates/teramindd/tests/detector_tool_chain.rs
git commit -m "feat(codify): tool_chain detector"
```

---

## Section 8 — Detector B: problem_fix

### Task 8.1: Failing test

**File:** Create `crates/teramindd/tests/detector_problem_fix.rs`.

- [ ] **Step 1**

```rust
//! Detector B — 4 sessions with `cargo test FAILED` user prompts + a follow-up
//! diff produce one observation.

use teramind_core::ids::{SessionId, TurnId};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::problem_fix;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_identical_failures_produce_one_observation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();

    for i in 0..4 {
        let sid = sessions.insert(NewSession {
            agent_id: agent.id, agent_session_id: None, cwd: "/proj",
            project_id: None, parent_session_id: None,
            git_head: None, git_branch: None,
            os: "linux", hostname: "h", user_login: "u",
            started_at: started, user_id: None, device_id: None,
        }).await?;
        let tid = trace.upsert_turn_with_id(
            TurnId(Uuid::new_v4()), sid, 0, started,
            Some(&format!("cargo test FAILED at file{i}.rs:42")),
        ).await?;
        trace.finalize_turn(tid, started, Some("Fixed."), None, Some("claude"), None, None).await?;
        diffs.insert(sid, tid, "src/lib.rs", "src/lib.rs",
                     teramind_core::types::Attribution::Agent, Some("rust"),
                     "old", "new",
                     "- pub fn foo() {}\n+ pub fn foo(x: i32) {}\n",
                     "h1", "h2", 100, started).await?;
    }

    let obs_repo = SkillObservationRepo::new(pool.clone());
    problem_fix::run(&pool, &obs_repo, time::Duration::days(30)).await?;

    let above = obs_repo.list_open(3, 10).await?;
    assert_eq!(above.len(), 1);
    assert_eq!(above[0].frequency, 4);
    assert_eq!(above[0].kind, "problem_fix");

    sup.shutdown().await?;
    Ok(())
}
```

(Adapt `DiffRepo::insert` to its real signature.)

Run: → FAIL.

### Task 8.2: Implement

**File:** Create `crates/teramindd/src/services/codify/detectors/problem_fix.rs`.

- [ ] **Step 1: Write the detector**

```rust
//! Detector B — repeated (error → fix) shapes.

use crate::services::codify::heuristics::{classify_diff, looks_like_error, normalize_error};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use teramind_core::ids::SessionId;
use teramind_db::pool::DbPool;
use teramind_db::repos::SkillObservationRepo;

pub async fn run(
    pool: &DbPool,
    obs: &SkillObservationRepo,
    window: time::Duration,
) -> anyhow::Result<()> {
    let cutoff = time::OffsetDateTime::now_utc() - window;

    // Pull turns + their associated diffs.
    let rows: Vec<(uuid::Uuid, uuid::Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT t.session_id, t.id AS turn_id, t.user_prompt,
               (SELECT string_agg(d.unified_diff, '\n') FROM file_diffs d WHERE d.turn_id = t.id) AS diff_agg
        FROM   turns t
        WHERE  t.started_at >= $1
          AND  t.user_prompt IS NOT NULL
        "#)
        .bind(cutoff).fetch_all(pool.pg()).await?;

    let mut sig_to_sessions: HashMap<String, Vec<SessionId>> = HashMap::new();
    let mut sig_to_context: HashMap<String, (String, String)> = HashMap::new();

    for (sid, _tid, prompt_opt, diff_opt) in rows {
        let Some(prompt) = prompt_opt else { continue; };
        let Some(diff) = diff_opt else { continue; };
        if !looks_like_error(&prompt) { continue; }

        let normalized = normalize_error(&prompt);
        let diff_kind = classify_diff(&diff).as_str();

        let mut h = Sha256::new();
        h.update(normalized.as_bytes());
        h.update(b"\x00");
        h.update(diff_kind.as_bytes());
        let sig = hex::encode(&h.finalize()[..8]);

        sig_to_sessions.entry(sig.clone()).or_default().push(SessionId(sid));
        sig_to_context.entry(sig).or_insert((normalized, diff_kind.to_string()));
    }

    for (sig, sessions) in sig_to_sessions {
        let (err, dk) = sig_to_context.get(&sig).cloned().unwrap_or_default();
        let context = serde_json::json!({ "error": err, "diff_kind": dk });
        obs.upsert("problem_fix", &sig, &sessions, context).await?;
    }
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramindd --test detector_problem_fix -- --test-threads=1`

Expected: 1 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/codify/detectors/problem_fix.rs \
        crates/teramindd/tests/detector_problem_fix.rs
git commit -m "feat(codify): problem_fix detector"
```

---

## Section 9 — `CodifyConfig` + Null provider + prompts

### Task 9.1: CodifyConfig

**File:** Modify `crates/teramindd/src/config.rs`.

- [ ] **Step 1: Add the struct**

Append:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct CodifyConfig {
    #[serde(default = "CodifyConfig::default_provider")]
    pub provider: String,
    #[serde(default = "CodifyConfig::default_model")]
    pub model: String,
    #[serde(default = "CodifyConfig::default_input_char_budget")]
    pub input_char_budget: usize,
    #[serde(default = "CodifyConfig::default_output_token_budget")]
    pub output_token_budget: u32,
    #[serde(default = "CodifyConfig::default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "CodifyConfig::default_autonomous_cycle_secs")]
    pub autonomous_cycle_secs: u64,
    #[serde(default = "CodifyConfig::default_min_observation_frequency")]
    pub min_observation_frequency: i32,
    #[serde(default = "CodifyConfig::default_max_pending_candidates")]
    pub max_pending_candidates: i64,
    #[serde(default = "CodifyConfig::default_digest_top_k")]
    pub digest_top_k: usize,
    #[serde(default)]
    pub detectors: DetectorToggles,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DetectorToggles {
    #[serde(default = "always_true")] pub tool_chain: bool,
    #[serde(default = "always_true")] pub problem_fix: bool,
    #[serde(default = "always_true")] pub llm_proposal: bool,
}

fn always_true() -> bool { true }

impl Default for DetectorToggles {
    fn default() -> Self {
        Self { tool_chain: true, problem_fix: true, llm_proposal: true }
    }
}

impl CodifyConfig {
    fn default_provider() -> String { "ollama".into() }
    fn default_model() -> String { "qwen3.6:latest".into() }
    fn default_input_char_budget() -> usize { 24_000 }
    fn default_output_token_budget() -> u32 { 1500 }
    fn default_poll_interval_secs() -> u64 { 30 }
    fn default_autonomous_cycle_secs() -> u64 { 21_600 }
    fn default_min_observation_frequency() -> i32 { 3 }
    fn default_max_pending_candidates() -> i64 { 50 }
    fn default_digest_top_k() -> usize { 5 }

    pub fn load_or_default(path: &std::path::Path) -> Self {
        if !path.exists() {
            return Self {
                provider: Self::default_provider(),
                model: Self::default_model(),
                input_char_budget: Self::default_input_char_budget(),
                output_token_budget: Self::default_output_token_budget(),
                poll_interval_secs: Self::default_poll_interval_secs(),
                autonomous_cycle_secs: Self::default_autonomous_cycle_secs(),
                min_observation_frequency: Self::default_min_observation_frequency(),
                max_pending_candidates: Self::default_max_pending_candidates(),
                digest_top_k: Self::default_digest_top_k(),
                detectors: Default::default(),
            };
        }
        let raw = std::fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to parse codify.toml; using defaults");
            Self::load_or_default(std::path::Path::new("/nonexistent"))
        })
    }
}
```

### Task 9.2: Null provider

**File:** Create `crates/teramindd/src/services/codify/null.rs`.

- [ ] **Step 1**

```rust
//! Null codify provider — for tests and the `provider = "null"` opt-out path.

use async_trait::async_trait;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};

pub struct NullCodifyProvider;

#[async_trait]
impl CodifyProvider for NullCodifyProvider {
    async fn codify(&self, _req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        Ok(CodifyResult {
            decision: CodifyDecision::Skip { reason: "null provider".into() },
            input_tokens: 0,
            output_tokens: 0,
        })
    }
    fn name(&self) -> &str { "null" }
}
```

### Task 9.3: Prompt module + snapshot

**File:** Create `crates/teramindd/src/services/codify/prompts.rs`.

- [ ] **Step 1**

```rust
//! System prompt for the codifier LLM. Snapshot-tested for change visibility.

pub const SYSTEM_PROMPT: &str = r#"You are a skill codifier. Given a repeated pattern observed across multiple AI-coding sessions, decide whether it's worth turning into a reusable skill that a future session could read at SessionStart.

A skill is worth codifying when:
- The pattern recurs deliberately (not coincidentally).
- The recipe is transferable — it would apply to other sessions in similar projects.
- Writing it down saves the next session at least a few turns of re-derivation.

Reject patterns that are:
- Trivial (one tool call, no decision).
- Project-specific in a way that doesn't generalize.
- Already well-known (basic git, cargo, npm).

Output strict JSON. Either:
  {"decision":"skip","reason":"..."}
OR:
  {"decision":"skill","name":"kebab-case","description":"one line","body":"# Markdown ...","applies_to_cwds":["/path/prefix"]}

Constraints:
- `name`: ≤60 chars, kebab-case, no spaces.
- `description`: ≤200 chars, one line.
- `body`: ≥200 chars, ≤4000 chars, valid Markdown.
- `body` MUST open with a frontmatter block:
---
source: codified
seeded_from: <N> sessions
first_observed: <YYYY-MM-DD>
applies_to: <cwd-pattern>
---
- `applies_to_cwds`: list of absolute path prefixes or globs (`*` allowed in segments). Empty list ⇒ global."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_mentions_strict_json_and_constraints() {
        assert!(SYSTEM_PROMPT.contains("strict JSON"));
        assert!(SYSTEM_PROMPT.contains("kebab-case"));
        assert!(SYSTEM_PROMPT.contains("applies_to_cwds"));
        assert!(SYSTEM_PROMPT.contains("frontmatter"));
    }
}
```

### Task 9.4: Register in codify/mod.rs + verify

- [ ] **Step 1**

Modify `crates/teramindd/src/services/codify/mod.rs`:

```rust
//! Skill codifier subsystem.

pub mod detectors;
pub mod glob;
pub mod heuristics;
pub mod null;
pub mod prompts;
```

- [ ] **Step 2: Run**

```bash
cargo test -p teramindd services::codify::
cargo clippy -p teramindd --all-targets -- -D warnings
```

Expected: all codify tests pass, clippy silent.

### Task 9.5: Commit

```bash
git add crates/teramindd/src/config.rs \
        crates/teramindd/src/services/codify/null.rs \
        crates/teramindd/src/services/codify/prompts.rs \
        crates/teramindd/src/services/codify/mod.rs
git commit -m "feat(codify): CodifyConfig + NullCodifyProvider + prompts"
```

---

## Section 10 — Ollama codify provider

### Task 10.1: Implement

**File:** Create `crates/teramindd/src/services/codify/ollama.rs`.

- [ ] **Step 1**

```rust
//! Ollama-backed codify provider. Reuses the same HTTP shape Plan H's
//! OllamaChatProvider uses (POST /api/chat with non-streaming JSON output).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};

pub struct OllamaCodifyProvider {
    base_url: String,
    model: String,
    http: reqwest::Client,
}

impl OllamaCodifyProvider {
    pub fn new(model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build().expect("reqwest client");
        Self { base_url: "http://localhost:11434".into(), model, http }
    }
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: ChatMessage,
    #[serde(default)] prompt_eval_count: u32,
    #[serde(default)] eval_count: u32,
}

#[derive(Deserialize)]
struct ChatMessage { content: String }

#[async_trait]
impl CodifyProvider for OllamaCodifyProvider {
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        use crate::services::codify::prompts::SYSTEM_PROMPT;
        let user_prompt = format!(
            "Observation kind: {}\nFrequency: {}\nProject cwds: {:?}\n\nBundled context:\n---\n{}\n---\n\nReturn JSON now.",
            req.observation_kind, req.frequency, req.cwds, req.bundled_context,
        );
        let body = json!({
            "model": self.model,
            "stream": false,
            "format": "json",
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user",   "content": user_prompt }
            ],
            "options": { "num_predict": req.max_output_tokens as i32 }
        });

        let resp = self.http.post(format!("{}/api/chat", self.base_url))
            .json(&body).send().await?
            .error_for_status()?;
        let parsed: OllamaChatResponse = resp.json().await?;
        let decision = parse_decision(&parsed.message.content)?;
        Ok(CodifyResult {
            decision,
            input_tokens: parsed.prompt_eval_count,
            output_tokens: parsed.eval_count,
        })
    }
    fn name(&self) -> &str { "ollama" }
}

fn parse_decision(raw: &str) -> anyhow::Result<CodifyDecision> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| anyhow::anyhow!("non-JSON output: {e}"))?;
    let kind = v.get("decision").and_then(|d| d.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing decision field"))?;
    match kind {
        "skip" => Ok(CodifyDecision::Skip {
            reason: v.get("reason").and_then(|r| r.as_str()).unwrap_or("").to_string(),
        }),
        "skill" => Ok(CodifyDecision::Skill {
            name: v["name"].as_str().ok_or_else(|| anyhow::anyhow!("missing name"))?.to_string(),
            description: v["description"].as_str().unwrap_or("").to_string(),
            body: v["body"].as_str().ok_or_else(|| anyhow::anyhow!("missing body"))?.to_string(),
            applies_to_cwds: v["applies_to_cwds"].as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        }),
        other => Err(anyhow::anyhow!("unknown decision: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::codify::CodifyDecision;

    #[test]
    fn parse_skip_round_trips() {
        let raw = r#"{"decision":"skip","reason":"trivial"}"#;
        match parse_decision(raw).unwrap() {
            CodifyDecision::Skip { reason } => assert_eq!(reason, "trivial"),
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn parse_skill_round_trips() {
        let raw = r#"{"decision":"skill","name":"rust-pr-prep","description":"d","body":"# x","applies_to_cwds":["/p"]}"#;
        match parse_decision(raw).unwrap() {
            CodifyDecision::Skill { name, body, applies_to_cwds, .. } => {
                assert_eq!(name, "rust-pr-prep");
                assert_eq!(body, "# x");
                assert_eq!(applies_to_cwds, vec!["/p".to_string()]);
            }
            _ => panic!("expected Skill"),
        }
    }

    #[test]
    fn parse_unknown_decision_errors() {
        let raw = r#"{"decision":"unknown"}"#;
        assert!(parse_decision(raw).is_err());
    }
}
```

- [ ] **Step 2: Register**

In `crates/teramindd/src/services/codify/mod.rs`, add `pub mod ollama;`.

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd services::codify::ollama::`

Expected: 3 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/codify/ollama.rs \
        crates/teramindd/src/services/codify/mod.rs
git commit -m "feat(codify): Ollama provider + JSON decision parser"
```

---

## Section 11 — Detector C: llm_proposal

### Task 11.1: Failing test (with NullProvider returning Skip)

**File:** Create `crates/teramindd/tests/detector_llm_proposal.rs`.

- [ ] **Step 1**

```rust
//! Detector C — calls a CodifyProvider once per cycle. With NullProvider it
//! always returns Skip, so no observation is emitted, but the call path
//! works end-to-end without panicking.

use std::sync::Arc;
use teramind_core::ids::TurnId;
use teramind_db::repos::{AgentRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::llm_proposal;
use teramindd::services::codify::null::NullCodifyProvider;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn null_provider_yields_no_observation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let agent = agents.upsert("claude_code", None).await?;
    for _ in 0..5 {
        let sid = sessions.insert(NewSession {
            agent_id: agent.id, agent_session_id: None, cwd: "/proj",
            project_id: None, parent_session_id: None,
            git_head: None, git_branch: None,
            os: "linux", hostname: "h", user_login: "u",
            started_at: started, user_id: None, device_id: None,
        }).await?;
        sessions.end(sid, started, "stop_hook").await?;
        let tid = trace.upsert_turn_with_id(TurnId(Uuid::new_v4()), sid, 0, started, Some("x")).await?;
        trace.finalize_turn(tid, started, Some("y"), None, None, None, None).await?;
    }

    let obs = SkillObservationRepo::new(pool.clone());
    let provider: Arc<dyn teramind_core::codify::CodifyProvider> = Arc::new(NullCodifyProvider);
    llm_proposal::run(&pool, &obs, provider.as_ref()).await?;

    assert!(obs.list_recent(Some("llm_proposal"), None, 10).await?.is_empty());

    sup.shutdown().await?;
    Ok(())
}
```

Run: → FAIL (`llm_proposal::run` missing).

### Task 11.2: Implement

**File:** Create `crates/teramindd/src/services/codify/detectors/llm_proposal.rs`.

- [ ] **Step 1**

```rust
//! Detector C — periodic LLM pass over recent sessions, no rule-based key.

use sha2::{Digest, Sha256};
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest};
use teramind_core::ids::SessionId;
use teramind_db::pool::DbPool;
use teramind_db::repos::SkillObservationRepo;

pub async fn run(
    pool: &DbPool,
    obs: &SkillObservationRepo,
    provider: &dyn CodifyProvider,
) -> anyhow::Result<()> {
    // Pick the 5 newest ended sessions.
    let rows: Vec<(uuid::Uuid, String, Option<time::OffsetDateTime>)> = sqlx::query_as(
        r#"SELECT id, cwd, ended_at
           FROM sessions
           WHERE ended_at IS NOT NULL
           ORDER BY ended_at DESC
           LIMIT 5"#)
        .fetch_all(pool.pg()).await?;
    if rows.is_empty() { return Ok(()); }

    // Bundle: wiki excerpts if any, else fall back to last few turns.
    let mut bundle = String::new();
    let mut session_ids: Vec<SessionId> = vec![];
    for (sid, cwd, _) in &rows {
        session_ids.push(SessionId(*sid));
        let wiki: Option<(String,)> = sqlx::query_as(
            r#"SELECT content FROM wiki_pages WHERE session_id = $1 ORDER BY generated_at DESC LIMIT 1"#)
            .bind(sid).fetch_optional(pool.pg()).await?;
        bundle.push_str(&format!("\n## session in {cwd}\n"));
        if let Some((c,)) = wiki {
            bundle.push_str(&c.chars().take(2000).collect::<String>());
        } else {
            bundle.push_str("(no wiki page)\n");
        }
    }

    let cwds: Vec<String> = rows.iter().map(|(_, c, _)| c.clone()).collect();
    let req = CodifyRequest {
        observation_kind: "llm_proposal".into(),
        bundled_context: bundle,
        frequency: rows.len() as u32,
        cwds,
        max_output_tokens: 600,
    };
    let result = provider.codify(req).await?;
    match result.decision {
        CodifyDecision::Skill { name, description, body: _, applies_to_cwds: _ } => {
            // Use the name as the dedup key so re-proposing the same name dedups.
            let mut h = Sha256::new(); h.update(name.as_bytes());
            let sig = hex::encode(&h.finalize()[..8]);
            let ctx = serde_json::json!({
                "proposed_name": name,
                "hint": description,
                "model": provider.name(),
            });
            obs.upsert("llm_proposal", &sig, &session_ids, ctx).await?;
        }
        CodifyDecision::Skip { reason: _ } => {
            // Don't insert anything — the LLM said nothing useful.
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Run**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramindd --test detector_llm_proposal -- --test-threads=1
```

Expected: 1 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/codify/detectors/llm_proposal.rs \
        crates/teramindd/tests/detector_llm_proposal.rs
git commit -m "feat(codify): llm_proposal detector"
```

---

## Section 12 — Synthesis + Promotion

### Task 12.1: Synthesis bundler

**File:** Create `crates/teramindd/src/services/codify/synthesis.rs`.

- [ ] **Step 1: Write the module**

```rust
//! Bundles context for an observation and calls the CodifyProvider.

use std::sync::Arc;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest};
use teramind_core::redact::Redactor;
use teramind_db::pool::DbPool;
use teramind_db::repos::{Candidate, Observation, SkillCandidateRepo, SkillObservationRepo};

pub struct SynthesisDeps {
    pub pool: DbPool,
    pub obs: SkillObservationRepo,
    pub cand: SkillCandidateRepo,
    pub provider: Arc<dyn CodifyProvider>,
    pub redactor: Arc<Redactor>,
    pub input_char_budget: usize,
    pub output_token_budget: u32,
    pub model_label: String,
}

pub async fn synthesize_one(deps: &SynthesisDeps, observation: Observation) -> anyhow::Result<Option<Candidate>> {
    let mut bundle = bundle_context(&deps.pool, &observation, deps.input_char_budget).await?;
    bundle = deps.redactor.apply(&bundle);

    let cwds = collect_cwds(&deps.pool, &observation.session_ids).await?;

    let req = CodifyRequest {
        observation_kind: observation.kind.clone(),
        bundled_context: bundle,
        frequency: observation.frequency as u32,
        cwds,
        max_output_tokens: deps.output_token_budget,
    };
    let result = deps.provider.codify(req).await?;

    match result.decision {
        CodifyDecision::Skip { reason } => {
            deps.obs.mark_skipped(observation.id, &reason).await?;
            Ok(None)
        }
        CodifyDecision::Skill { name, description, body, applies_to_cwds } => {
            let session_ids: Vec<teramind_core::ids::SessionId> = observation.session_ids.iter()
                .copied().map(teramind_core::ids::SessionId).collect();
            let cand_id = deps.cand.insert(
                observation.id, &name, &description, &body,
                &applies_to_cwds, &session_ids,
                &deps.model_label,
                result.input_tokens as i32, result.output_tokens as i32,
            ).await?;
            // Supersede any older pending candidates for the same observation.
            let _ = deps.cand.supersede_prior(observation.id, cand_id).await;
            deps.obs.mark_synthesized(observation.id).await?;
            Ok(Some(Candidate {
                id: cand_id,
                observation_id: observation.id,
                name, description, body,
                applies_to_cwds,
                source_session_ids: observation.session_ids.clone(),
                model: deps.model_label.clone(),
                input_tokens: result.input_tokens as i32,
                output_tokens: result.output_tokens as i32,
                generated_at: time::OffsetDateTime::now_utc(),
                status: "pending".into(),
                reviewer: None,
                reviewed_at: None,
            }))
        }
    }
}

async fn bundle_context(pool: &DbPool, obs: &Observation, budget: usize) -> anyhow::Result<String> {
    let mut out = format!("Observation kind: {}\nSignature: {}\nFrequency: {}\nContext: {}\n\n",
        obs.kind, obs.signature, obs.frequency, obs.context_blob);
    for sid in obs.session_ids.iter().take(5) {
        // Wiki excerpt first (most signal-dense).
        if let Some((content,)) = sqlx::query_as::<_, (String,)>(
            r#"SELECT content FROM wiki_pages WHERE session_id = $1 ORDER BY generated_at DESC LIMIT 1"#)
            .bind(sid).fetch_optional(pool.pg()).await?
        {
            out.push_str(&format!("\n## session {sid} — wiki\n"));
            out.push_str(&content.chars().take(3000).collect::<String>());
        } else {
            // Representative turns (up to 3) when no wiki exists.
            let turns: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
                r#"SELECT user_prompt, assistant_text
                   FROM turns WHERE session_id = $1
                   ORDER BY ordinal LIMIT 3"#)
                .bind(sid).fetch_all(pool.pg()).await?;
            out.push_str(&format!("\n## session {sid} — turns\n"));
            for (p, a) in turns {
                if let Some(p) = p { out.push_str(&format!("> {p}\n")); }
                if let Some(a) = a { out.push_str(&format!("{a}\n")); }
            }
        }
        if out.len() > budget {
            out.truncate(budget);
            out.push_str("\n…[truncated]");
            break;
        }
    }
    Ok(out)
}

async fn collect_cwds(pool: &DbPool, session_ids: &[uuid::Uuid]) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"SELECT DISTINCT cwd FROM sessions WHERE id = ANY($1)"#)
        .bind(session_ids).fetch_all(pool.pg()).await?;
    Ok(rows.into_iter().map(|(c,)| c).collect())
}
```

### Task 12.2: Promotion

**File:** Create `crates/teramindd/src/services/codify/promote.rs`.

- [ ] **Step 1**

```rust
//! Transactional promotion of approved candidates into the live skills table.

use teramind_db::pool::DbPool;
use teramind_db::repos::{SkillCandidateRepo, SkillRepo};
use tracing::{info, warn};

pub async fn promote_approved_batch(
    pool: &DbPool,
    candidates: &SkillCandidateRepo,
    skills: &SkillRepo,
    limit: i64,
) -> anyhow::Result<u64> {
    let approved = candidates.list_approved(limit).await?;
    let mut count = 0u64;
    for c in approved {
        let res = sqlx::query("BEGIN").execute(pool.pg()).await;
        if res.is_err() { continue; }

        let skill_res = skills.upsert_codified(
            &c.name, &c.description, &c.body,
            &c.source_session_ids,
            &c.applies_to_cwds,
        ).await;
        match skill_res {
            Ok(_) => {
                if let Err(e) = candidates.mark_promoted(c.id).await {
                    warn!(error = %e, candidate = %c.id.0, "mark_promoted failed; rolling back");
                    let _ = sqlx::query("ROLLBACK").execute(pool.pg()).await;
                    continue;
                }
                let _ = sqlx::query("COMMIT").execute(pool.pg()).await;
                info!(name = %c.name, "candidate promoted to skill");
                count += 1;
            }
            Err(e) => {
                warn!(error = %e, candidate = %c.id.0, "promotion upsert failed");
                let _ = sqlx::query("ROLLBACK").execute(pool.pg()).await;
            }
        }
    }
    Ok(count)
}
```

### Task 12.3: Register modules

- [ ] **Step 1**

In `crates/teramindd/src/services/codify/mod.rs`, add:

```rust
pub mod promote;
pub mod synthesis;
```

### Task 12.4: Verify + commit

```bash
cargo build -p teramindd
cargo clippy -p teramindd --all-targets -- -D warnings
git add crates/teramindd/src/services/codify/synthesis.rs \
        crates/teramindd/src/services/codify/promote.rs \
        crates/teramindd/src/services/codify/mod.rs
git commit -m "feat(codify): synthesis bundler + promotion"
```

---

## Section 13 — `codifier_worker`

### Task 13.1: Failing test (synthesize + promote loops end-to-end with NullProvider)

**File:** Create `crates/teramindd/tests/codifier_worker_e2e.rs`.

- [ ] **Step 1**

```rust
//! E2E with a mock CodifyProvider that returns a Skill: observation→candidate
//! → SQL-approve → next tick promotes it → skills row exists with source='codified'.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};
use teramind_core::ids::SessionId;
use teramind_core::redact::Redactor;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::config::CodifyConfig;
use teramindd::services::codifier_worker::{CodifierDeps, CodifierWorker};
use uuid::Uuid;

struct AlwaysSkill;

#[async_trait]
impl CodifyProvider for AlwaysSkill {
    async fn codify(&self, _: CodifyRequest) -> anyhow::Result<CodifyResult> {
        Ok(CodifyResult {
            decision: CodifyDecision::Skill {
                name: "test-skill".into(),
                description: "desc".into(),
                body: "---\nsource: codified\nseeded_from: 3 sessions\nfirst_observed: 2026-05-17\napplies_to: /proj\n---\n\n# test-skill\n\nbody body body".into(),
                applies_to_cwds: vec!["/proj".into()],
            },
            input_tokens: 100,
            output_tokens: 50,
        })
    }
    fn name(&self) -> &str { "always-skill" }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn synthesis_then_approval_promotes() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let obs = SkillObservationRepo::new(pool.clone());
    obs.upsert("tool_chain", "sig1",
               &[SessionId(Uuid::new_v4()), SessionId(Uuid::new_v4()), SessionId(Uuid::new_v4())],
               serde_json::json!({})).await?;

    let cand = SkillCandidateRepo::new(pool.clone());
    let skills = SkillRepo::new(pool.clone());

    let cfg = CodifyConfig::load_or_default(std::path::Path::new("/nonexistent"));
    let _w = CodifierWorker::spawn(CodifierDeps {
        pool: pool.clone(),
        obs: obs.clone(),
        cand: cand.clone(),
        skills: skills.clone(),
        provider: Arc::new(AlwaysSkill),
        redactor: Arc::new(Redactor::with_default_rules()),
        cfg,
        run_detectors: false,
        model_label: "mock".into(),
        poll_interval: Duration::from_millis(100),
    });

    // Wait for synthesis.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !cand.list_pending(10).await?.is_empty() { break; }
    }
    let pending = cand.list_pending(10).await?;
    assert_eq!(pending.len(), 1);
    let cid = pending[0].id;

    // Approve via SQL.
    sqlx::query("UPDATE skill_candidates SET status='approved', reviewer='admin', reviewed_at=now() WHERE id=$1")
        .bind(cid.0).execute(pool.pg()).await?;

    // Wait for promotion.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM skills WHERE source='codified'")
            .fetch_one(pool.pg()).await?;
        if n == 1 { break; }
    }
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM skills WHERE source='codified' AND name='test-skill'")
        .fetch_one(pool.pg()).await?;
    assert_eq!(n, 1, "candidate must be promoted");

    sup.shutdown().await?;
    Ok(())
}
```

Run: → FAIL.

### Task 13.2: Implement

**File:** Create `crates/teramindd/src/services/codifier_worker.rs`.

- [ ] **Step 1**

```rust
//! Three-loop orchestrator: synthesize + promote (poll) + detectors (long cycle).

use crate::config::CodifyConfig;
use crate::services::codify::detectors::{llm_proposal, problem_fix, tool_chain};
use crate::services::codify::promote::promote_approved_batch;
use crate::services::codify::synthesis::{synthesize_one, SynthesisDeps};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::codify::CodifyProvider;
use teramind_core::redact::Redactor;
use teramind_db::pool::DbPool;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};
use tracing::{info, warn};

#[derive(Default)]
pub struct CodifierStats {
    pub observations_total: AtomicU64,
    pub candidates_total:   AtomicU64,
    pub promotions_total:   AtomicU64,
    pub skips_total:        AtomicU64,
}

pub struct CodifierDeps {
    pub pool: DbPool,
    pub obs: SkillObservationRepo,
    pub cand: SkillCandidateRepo,
    pub skills: SkillRepo,
    pub provider: Arc<dyn CodifyProvider>,
    pub redactor: Arc<Redactor>,
    pub cfg: CodifyConfig,
    /// Test escape hatch: when false, the detector loop never runs.
    pub run_detectors: bool,
    pub model_label: String,
    pub poll_interval: Duration,
}

pub struct CodifierWorker {
    pub stats: Arc<CodifierStats>,
    _h_synth: tokio::task::JoinHandle<()>,
    _h_detect: Option<tokio::task::JoinHandle<()>>,
}

impl CodifierWorker {
    pub fn spawn(deps: CodifierDeps) -> Self {
        let stats = Arc::new(CodifierStats::default());

        let synth_deps = deps.clone_for_synth(stats.clone());
        let h_synth = tokio::spawn(async move { synthesis_promote_loop(synth_deps).await });

        let h_detect = if deps.run_detectors {
            let dep = deps.clone_for_detectors(stats.clone());
            Some(tokio::spawn(async move { detector_loop(dep).await }))
        } else { None };

        Self { stats, _h_synth: h_synth, _h_detect: h_detect }
    }
}

impl CodifierDeps {
    fn clone_for_synth(&self, _stats: Arc<CodifierStats>) -> SynthAndPromoteLoop {
        SynthAndPromoteLoop {
            pool: self.pool.clone(),
            obs: self.obs.clone(),
            cand: self.cand.clone(),
            skills: self.skills.clone(),
            provider: self.provider.clone(),
            redactor: self.redactor.clone(),
            cfg: self.cfg.clone(),
            model_label: self.model_label.clone(),
            poll_interval: self.poll_interval,
        }
    }
    fn clone_for_detectors(&self, _stats: Arc<CodifierStats>) -> DetectorLoop {
        DetectorLoop {
            pool: self.pool.clone(),
            obs: self.obs.clone(),
            provider: self.provider.clone(),
            cfg: self.cfg.clone(),
        }
    }
}

struct SynthAndPromoteLoop {
    pool: DbPool,
    obs: SkillObservationRepo,
    cand: SkillCandidateRepo,
    skills: SkillRepo,
    provider: Arc<dyn CodifyProvider>,
    redactor: Arc<Redactor>,
    cfg: CodifyConfig,
    model_label: String,
    poll_interval: Duration,
}

async fn synthesis_promote_loop(d: SynthAndPromoteLoop) {
    loop {
        // 1. Promote any approved candidates.
        if let Err(e) = promote_approved_batch(&d.pool, &d.cand, &d.skills, 10).await {
            warn!(error = %e, "promote_approved_batch error");
        }

        // 2. Back-pressure: skip synthesis when too many pending.
        let pending_n = d.cand.list_pending(d.cfg.max_pending_candidates).await
            .map(|v| v.len() as i64).unwrap_or(0);
        if pending_n >= d.cfg.max_pending_candidates {
            tokio::time::sleep(d.poll_interval).await;
            continue;
        }

        // 3. Pick one open observation above threshold.
        let open = d.obs.list_open(d.cfg.min_observation_frequency, 1).await.ok().unwrap_or_default();
        if let Some(o) = open.into_iter().next() {
            let deps = SynthesisDeps {
                pool: d.pool.clone(),
                obs: d.obs.clone(),
                cand: d.cand.clone(),
                provider: d.provider.clone(),
                redactor: d.redactor.clone(),
                input_char_budget: d.cfg.input_char_budget,
                output_token_budget: d.cfg.output_token_budget,
                model_label: d.model_label.clone(),
            };
            match synthesize_one(&deps, o).await {
                Ok(Some(_)) => info!("synthesized candidate"),
                Ok(None)    => info!("observation skipped"),
                Err(e)      => warn!(error = %e, "synthesis error"),
            }
        }

        tokio::time::sleep(d.poll_interval).await;
    }
}

struct DetectorLoop {
    pool: DbPool,
    obs: SkillObservationRepo,
    provider: Arc<dyn CodifyProvider>,
    cfg: CodifyConfig,
}

async fn detector_loop(d: DetectorLoop) {
    loop {
        if d.cfg.detectors.tool_chain {
            if let Err(e) = tool_chain::run(&d.pool, &d.obs, time::Duration::days(30)).await {
                warn!(error = %e, "tool_chain detector error");
            }
        }
        if d.cfg.detectors.problem_fix {
            if let Err(e) = problem_fix::run(&d.pool, &d.obs, time::Duration::days(30)).await {
                warn!(error = %e, "problem_fix detector error");
            }
        }
        if d.cfg.detectors.llm_proposal {
            if let Err(e) = llm_proposal::run(&d.pool, &d.obs, d.provider.as_ref()).await {
                warn!(error = %e, "llm_proposal detector error");
            }
        }
        tokio::time::sleep(Duration::from_secs(d.cfg.autonomous_cycle_secs)).await;
    }
}
```

- [ ] **Step 2: Register**

In `crates/teramindd/src/services/mod.rs`, add `pub mod codifier_worker;`.

- [ ] **Step 3: Run**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramindd --test codifier_worker_e2e -- --test-threads=1
```

Expected: 1 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/codifier_worker.rs \
        crates/teramindd/src/services/mod.rs \
        crates/teramindd/tests/codifier_worker_e2e.rs
git commit -m "feat(codify): codifier_worker (3-loop orchestrator)"
```

---

## Section 14 — IPC contract + dispatch additions

### Task 14.1: Add Request variants

**File:** Modify `crates/teramind-ipc/src/proto.rs`.

- [ ] **Step 1**

Add to the `Request` enum:

```rust
    CodifyNow {
        seed_session_ids: Vec<String>,
        hint: Option<String>,
    },
    SkillsList {
        filter: Option<String>,   // "all" | "pending" | "rejected" | "approved" | "codified" | "authored"
        limit: u32,
    },
    SkillsShow {
        name_or_id: String,
    },
    SkillsObservations {
        kind: Option<String>,
        min_freq: i32,
        status: Option<String>,
        limit: u32,
    },
```

Add to the `Response` enum:

```rust
    CodifyQueued {
        observation_id: String,
    },
    SkillsList {
        rows: Vec<SkillRow>,
    },
    SkillShow {
        name: String,
        description: String,
        body: String,
        source: String,
        applies_to_cwds: Vec<String>,
    },
    SkillsObservations {
        rows: Vec<ObservationRow>,
    },
```

Add the row structs at the end of `proto.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,        // "authored" | "codified" | "imported"
    pub status: Option<String>, // None for live skills, Some("pending"|...) for candidates
    pub applies_to_cwds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationRow {
    pub id: String,
    pub kind: String,
    pub signature: String,
    pub frequency: i32,
    pub status: String,
    pub last_seen_at: String,
}
```

### Task 14.2: Dispatch the new variants in rpc_dispatch.rs

**File:** Modify `crates/teramindd/src/services/rpc_dispatch.rs`.

- [ ] **Step 1: Extend `RpcDeps`**

Add fields:

```rust
pub skill_obs: teramind_db::repos::SkillObservationRepo,
pub skill_cand: teramind_db::repos::SkillCandidateRepo,
pub min_observation_frequency: i32,
```

- [ ] **Step 2: Add match arms for the new variants**

Inside `dispatch`'s match, add:

```rust
Request::CodifyNow { seed_session_ids, hint } => {
    use sha2::{Digest, Sha256};
    let sids: Vec<teramind_core::ids::SessionId> = seed_session_ids.iter()
        .filter_map(|s| uuid::Uuid::parse_str(s).ok())
        .map(teramind_core::ids::SessionId)
        .collect();
    let hint_str = hint.unwrap_or_default();
    let mut h = Sha256::new(); h.update(hint_str.as_bytes()); h.update(format!("{:?}", sids).as_bytes());
    let sig = hex::encode(&h.finalize()[..8]);
    let ctx = serde_json::json!({ "hint": hint_str, "source": "mcp" });
    let _ = deps.skill_obs.upsert("llm_proposal", &sig,
        if sids.is_empty() { &[] } else { &sids[..] }, ctx).await;
    let obs = deps.skill_obs.find_by_sig("llm_proposal", &sig).await.ok().flatten();
    let id = obs.map(|o| o.id.0.to_string()).unwrap_or_default();
    Response::CodifyQueued { observation_id: id }
}

Request::SkillsList { filter, limit } => {
    let mut rows: Vec<teramind_ipc::proto::SkillRow> = vec![];
    let f = filter.unwrap_or_else(|| "all".into());
    if f == "pending" || f == "rejected" || f == "approved" {
        let cands = deps.skill_cand.list_filter(Some(&f), limit as i64).await.unwrap_or_default();
        for c in cands {
            rows.push(teramind_ipc::proto::SkillRow {
                id: c.id.0.to_string(),
                name: c.name, description: c.description,
                source: "candidate".into(),
                status: Some(c.status),
                applies_to_cwds: c.applies_to_cwds,
            });
        }
    } else {
        // Live skills.
        let live: Vec<(uuid::Uuid, String, String, String, Vec<String>)> = sqlx::query_as(
            r#"SELECT id, name, description, source, applies_to_cwds
               FROM skills ORDER BY updated_at DESC LIMIT $1"#)
            .bind(limit as i64).fetch_all(deps.pool.pg()).await.unwrap_or_default();
        for (id, n, d, s, cwds) in live {
            if f == "codified" && s != "codified" { continue; }
            if f == "authored" && s != "authored" { continue; }
            rows.push(teramind_ipc::proto::SkillRow {
                id: id.to_string(), name: n, description: d,
                source: s, status: None, applies_to_cwds: cwds,
            });
        }
    }
    Response::SkillsList { rows }
}

Request::SkillsShow { name_or_id } => {
    let row: Option<(uuid::Uuid, String, String, String, String, Vec<String>)> = sqlx::query_as(
        r#"SELECT id, name, description, body, source, applies_to_cwds
           FROM skills
           WHERE name = $1 OR id::text = $1"#)
        .bind(&name_or_id).fetch_optional(deps.pool.pg()).await.unwrap_or(None);
    if let Some((_, name, description, body, source, applies_to_cwds)) = row {
        Response::SkillShow { name, description, body, source, applies_to_cwds }
    } else {
        Response::Error(format!("no skill named or with id '{}'", name_or_id))
    }
}

Request::SkillsObservations { kind, min_freq, status, limit } => {
    let _ = min_freq; // status takes priority; could combine
    let obs = deps.skill_obs.list_recent(kind.as_deref(), status.as_deref(), limit as i64)
        .await.unwrap_or_default();
    let rows = obs.into_iter().map(|o| teramind_ipc::proto::ObservationRow {
        id: o.id.0.to_string(),
        kind: o.kind, signature: o.signature, frequency: o.frequency,
        status: o.status, last_seen_at: o.last_seen_at.to_string(),
    }).collect();
    Response::SkillsObservations { rows }
}
```

- [ ] **Step 3: Update `RpcDeps` factory sites**

Two call sites build `RpcDeps`:
- `crates/teramindd/src/services/ipc_server.rs::DaemonIpcHandler::rpc_deps()`
- `crates/teramind-sync-server/src/state.rs::AppState::rpc_deps()`

Both gain three fields:

```rust
skill_obs: teramind_db::repos::SkillObservationRepo::new(<pool>.clone()),
skill_cand: teramind_db::repos::SkillCandidateRepo::new(<pool>.clone()),
min_observation_frequency: 3,   // default; server can read CodifyConfig later
```

(Plumbing the live `min_observation_frequency` from `CodifyConfig` to `AppState` is a small enrichment in §17 if needed.)

- [ ] **Step 4: Update `ipc_server.rs`'s combined match arm**

The combined arm from Plan K (`req @ (Request::Search(_) | Request::Recall(_) | ...)`) needs to include the four new variants:

```rust
req @ (Request::Search(_) | Request::Recall(_) | Request::AutoRecall(_)
       | Request::SaveSkill(_) | Request::WikiLookup { .. }
       | Request::CodifyNow { .. } | Request::SkillsList { .. }
       | Request::SkillsShow { .. } | Request::SkillsObservations { .. }) => {
    crate::services::rpc_dispatch::dispatch(&self.rpc_deps(), req, None).await
}
```

### Task 14.3: Verify + commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramindd -- --test-threads=1
```

All existing tests stay green.

```bash
git add crates/teramind-ipc/src/proto.rs \
        crates/teramindd/src/services/rpc_dispatch.rs \
        crates/teramindd/src/services/ipc_server.rs \
        crates/teramind-sync-server/src/state.rs
git commit -m "feat(ipc): CodifyNow + SkillsList/Show/Observations dispatch"
```

---

## Section 15 — MCP `mcp__teramind__codify` tool

### Task 15.1: Implement

**File:** Modify `crates/teramind-mcp/src/server.rs`.

- [ ] **Step 1: Add tool**

Add a new tool body alongside the existing five:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CodifyArgs {
    /// Sessions to seed the proposal. Empty = let the daemon pick recent.
    #[serde(default)]
    pub seed_session_ids: Vec<String>,
    /// Optional one-line hint about the pattern.
    pub hint: Option<String>,
}

// inside #[tool_router] impl …
/// Queue a codifier proposal for an observed pattern. Returns immediately;
/// the candidate appears later in `mcp__teramind__search` after admin approval.
#[tool]
async fn codify(
    &self,
    Parameters(args): Parameters<CodifyArgs>,
) -> Result<CallToolResult, McpError> {
    let req = Request::CodifyNow {
        seed_session_ids: args.seed_session_ids,
        hint: args.hint,
    };
    let resp = self.ipc_request(req).await?;
    let body = match resp {
        Response::CodifyQueued { observation_id } => serde_json::json!({
            "queued": true, "observation_id": observation_id,
        }),
        Response::Error(e) => return Err(McpError::internal_error(e, None)),
        other => return Err(McpError::internal_error(format!("unexpected: {other:?}"), None)),
    };
    Ok(CallToolResult::success(vec![
        Content::text(serde_json::to_string_pretty(&body).unwrap_or_default()),
    ]))
}
```

### Task 15.2: Test

**File:** Create `crates/teramind-mcp/tests/codify_tool.rs`.

- [ ] **Step 1**

Follow the same pattern as `tests/team_share_set.rs`: spin up a recording IPC server, invoke the `codify` tool, assert the recorded request matches `Request::CodifyNow { seed_session_ids: [...], hint: Some("...") }`.

(Use the existing test scaffold from `tests/team_share_set.rs` — copy its setup; only the tool name + args + recorded-request shape change.)

### Task 15.3: Verify + commit

```bash
cargo test -p teramind-mcp --test codify_tool
cargo clippy -p teramind-mcp --all-targets -- -D warnings
git add crates/teramind-mcp/src/server.rs crates/teramind-mcp/tests/codify_tool.rs
git commit -m "feat(mcp): mcp__teramind__codify tool"
```

---

## Section 16 — CLI `teramind skills`

### Task 16.1: Add subcommand

**File:** Modify `crates/teramind/src/cli.rs`.

- [ ] **Step 1**

Add to the `Command` enum:

```rust
/// Inspect skills + codifier observations.
Skills {
    #[command(subcommand)]
    action: SkillsAction,
},
```

After the enum:

```rust
#[derive(Subcommand)]
pub enum SkillsAction {
    /// List skills.
    List {
        /// Filter: all | pending | approved | rejected | codified | authored.
        #[arg(long, default_value = "all")]
        filter: String,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Print one skill's full body.
    Show { name_or_id: String },
    /// List observations (for debugging).
    Observations {
        #[arg(long)] kind: Option<String>,
        #[arg(long, default_value = "0")] min_freq: i32,
        #[arg(long)] status: Option<String>,
        #[arg(long, default_value = "50")] limit: u32,
    },
}
```

### Task 16.2: Implement

**File:** Create `crates/teramind/src/commands/skills.rs`.

- [ ] **Step 1**

```rust
//! `teramind skills list / show / observations`.

use anyhow::Result;
use teramind_ipc::proto::{Request, Response};

pub async fn list(filter: String, limit: u32) -> Result<()> {
    let resp = crate::ipc::send(Request::SkillsList { filter: Some(filter), limit }).await?;
    match resp {
        Response::SkillsList { rows } => {
            if rows.is_empty() { println!("(no skills)"); return Ok(()); }
            println!("{:<36}  {:<10}  {:<30}  description", "id", "source", "name");
            for r in rows {
                let src = match r.status {
                    Some(s) => format!("candidate({s})"),
                    None    => r.source,
                };
                println!("{:<36}  {:<10}  {:<30}  {}", r.id, src, r.name, r.description);
            }
        }
        Response::Error(e) => return Err(anyhow::anyhow!(e)),
        other => return Err(anyhow::anyhow!("unexpected response: {other:?}")),
    }
    Ok(())
}

pub async fn show(name_or_id: String) -> Result<()> {
    let resp = crate::ipc::send(Request::SkillsShow { name_or_id }).await?;
    match resp {
        Response::SkillShow { name, description, body, source, applies_to_cwds } => {
            println!("# {name}");
            println!("source: {source}");
            println!("applies_to_cwds: {applies_to_cwds:?}");
            println!("description: {description}");
            println!();
            println!("{body}");
        }
        Response::Error(e) => return Err(anyhow::anyhow!(e)),
        other => return Err(anyhow::anyhow!("unexpected response: {other:?}")),
    }
    Ok(())
}

pub async fn observations(kind: Option<String>, min_freq: i32, status: Option<String>, limit: u32) -> Result<()> {
    let resp = crate::ipc::send(Request::SkillsObservations { kind, min_freq, status, limit }).await?;
    match resp {
        Response::SkillsObservations { rows } => {
            if rows.is_empty() { println!("(no observations)"); return Ok(()); }
            println!("{:<36}  {:<14}  {:>5}  {:<14}  signature", "id", "kind", "freq", "status");
            for r in rows {
                println!("{:<36}  {:<14}  {:>5}  {:<14}  {}", r.id, r.kind, r.frequency, r.status, r.signature);
            }
        }
        Response::Error(e) => return Err(anyhow::anyhow!(e)),
        other => return Err(anyhow::anyhow!("unexpected response: {other:?}")),
    }
    Ok(())
}
```

Note: `crate::ipc::send` is whatever helper the existing CLI commands use to dispatch via `RpcTransport`. Inspect `crates/teramind/src/commands/search.rs` for the pattern and adapt.

### Task 16.3: Register

**Files:**
- `crates/teramind/src/commands/mod.rs`: `pub mod skills;`
- `crates/teramind/src/main.rs`: dispatch the new variant:

```rust
Command::Skills { action } => match action {
    SkillsAction::List { filter, limit } => commands::skills::list(filter, limit).await,
    SkillsAction::Show { name_or_id } => commands::skills::show(name_or_id).await,
    SkillsAction::Observations { kind, min_freq, status, limit } =>
        commands::skills::observations(kind, min_freq, status, limit).await,
},
```

### Task 16.4: Smoke test

**File:** Create `crates/teramind/tests/skills_cli.rs`.

- [ ] **Step 1**

```rust
//! Smoke test — call commands::skills::list against an embedded daemon.
//! (Pattern: same as Plan K's two_dev_team_mode.rs and Plan L's feed_cli.rs —
//! launch a local IPC server with a mock handler, set TERAMIND_SOCKET, call
//! the lib fn directly, assert non-error completion.)

use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_runs_without_panic_on_empty_db() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    // Launch the daemon IPC handler on a temp UDS, set TERAMIND_SOCKET env,
    // then call teramind_cli::commands::skills::list. The exact wiring
    // depends on how the existing `teramind search` test bootstraps —
    // copy that pattern.
    //
    // Empty DB → empty list → no error.
    sup.shutdown().await?;
    Ok(())
}
```

(This is intentionally a sketch — the daemon-launch boilerplate comes from `crates/teramind/tests/search_cli.rs` if it exists, otherwise from Plan J's `init_team.rs`.)

### Task 16.5: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind --test skills_cli -- --test-threads=1
git add crates/teramind/src/cli.rs \
        crates/teramind/src/commands/skills.rs \
        crates/teramind/src/commands/mod.rs \
        crates/teramind/src/main.rs \
        crates/teramind/tests/skills_cli.rs
git commit -m "feat(cli): teramind skills list / show / observations"
```

---

## Section 17 — Auto-recall digest extension + app wiring + doctor

### Task 17.1: Extend `do_auto_recall`

**File:** Modify `crates/teramindd/src/services/search.rs`.

- [ ] **Step 1: Add the helper**

```rust
/// Returns the "Relevant codified skills" section of the SessionStart digest
/// (or empty string if no skills match the cwd).
pub async fn relevant_codified_skills(
    skills: &teramind_db::repos::SkillRepo,
    cwd: &str,
    top_k: usize,
) -> String {
    let rows = match skills.list_codified_for_cwd(cwd, 50).await {
        Ok(rs) => rs,
        Err(_) => return String::new(),
    };
    let matched: Vec<_> = rows.into_iter()
        .filter(|(_, _, _, applies, _)| crate::services::codify::glob::matches_any(applies, cwd))
        .take(top_k)
        .collect();
    if matched.is_empty() { return String::new(); }
    let mut out = String::from("\n## Relevant codified skills\n\n");
    for (_id, name, desc, _, seeded_from) in &matched {
        out.push_str(&format!(
            "- **{name}** — {desc} _(seeded from {seeded_from} sessions)_\n"
        ));
    }
    out.push_str("\nTo recall the full body of any skill: `mcp__teramind__search` with the skill name.\n");
    out
}
```

- [ ] **Step 2: Wire into `do_auto_recall`**

Locate the spot in `do_auto_recall` where the markdown digest is assembled. Before the existing "recent sessions" / "latest wiki" sections, prepend:

```rust
out.push_str(&relevant_codified_skills(&skills_repo, &req.cwd, 5).await);
```

The `skills_repo` may need to be added to `do_auto_recall`'s parameter list — inspect the existing fn signature first.

### Task 17.2: Add Codify wiring in app.rs

**File:** Modify `crates/teramindd/src/app.rs`.

- [ ] **Step 1**

After Plan H's summarizer worker spawn, add:

```rust
let codify_cfg_path = paths.config_dir.join("codify.toml");
let codify_cfg = crate::config::CodifyConfig::load_or_default(&codify_cfg_path);
let codify_provider: std::sync::Arc<dyn teramind_core::codify::CodifyProvider> =
    match codify_cfg.provider.as_str() {
        "null" => std::sync::Arc::new(crate::services::codify::null::NullCodifyProvider),
        "ollama" => std::sync::Arc::new(
            crate::services::codify::ollama::OllamaCodifyProvider::new(codify_cfg.model.clone())
        ),
        // anthropic is added in §18; for v1 of the implementation, fall through to null.
        _ => std::sync::Arc::new(crate::services::codify::null::NullCodifyProvider),
    };
let _codifier = crate::services::codifier_worker::CodifierWorker::spawn(
    crate::services::codifier_worker::CodifierDeps {
        pool: pool.clone(),
        obs: teramind_db::repos::SkillObservationRepo::new(pool.clone()),
        cand: teramind_db::repos::SkillCandidateRepo::new(pool.clone()),
        skills: teramind_db::repos::SkillRepo::new(pool.clone()),
        provider: codify_provider.clone(),
        redactor: redactor.clone(),
        cfg: codify_cfg.clone(),
        run_detectors: true,
        model_label: format!("{}:{}", codify_cfg.provider, codify_cfg.model),
        poll_interval: std::time::Duration::from_secs(codify_cfg.poll_interval_secs),
    },
);
```

`redactor` is the existing `Arc<Redactor>` Plan A spawns. Reuse it.

### Task 17.3: Extend doctor

**File:** Modify `crates/teramind/src/commands/doctor.rs`.

- [ ] **Step 1**

After the existing team-mode block, add:

```rust
let codify_cfg_path = paths.config_dir.join("codify.toml");
if codify_cfg_path.exists() {
    let raw = std::fs::read_to_string(&codify_cfg_path).unwrap_or_default();
    let provider = if raw.contains("provider = \"null\"") { "null" }
                   else if raw.contains("provider = \"anthropic\"") { "anthropic" }
                   else { "ollama" };
    println!("codifier:    enabled ({})", provider);
} else {
    println!("codifier:    disabled (no codify.toml)");
}
```

(Real backlog numbers would come from the IPC `StatusReport` — extending that struct is a small follow-up; v1 just shows the configured state.)

### Task 17.4: Verify

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramindd -- --test-threads=1
./target/debug/teramind doctor 2>&1 | grep -E 'codifier|team mode' || true
```

### Task 17.5: Commit

```bash
git add crates/teramindd/src/services/search.rs \
        crates/teramindd/src/app.rs \
        crates/teramind/src/commands/doctor.rs
git commit -m "feat(codify): auto-recall digest + app wiring + doctor surface"
```

---

## Section 18 — Anthropic provider (gated)

### Task 18.1: Implement

**File:** Create `crates/teramindd/src/services/codify/anthropic.rs`.

- [ ] **Step 1**

Mirror Plan H's `summarize::anthropic::AnthropicProvider` shape. Construction refuses unless `secrets.toml` has `network_egress = true` and `anthropic_api_key` is present:

```rust
//! Anthropic codify provider (gated by network_egress=true + anthropic_api_key).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};

pub struct AnthropicCodifyProvider {
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl AnthropicCodifyProvider {
    pub fn try_new(secrets_path: &Path, model: String) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Secrets {
            #[serde(default)] network_egress: bool,
            anthropic_api_key: Option<String>,
        }
        if !secrets_path.exists() {
            anyhow::bail!("Anthropic codify provider requires {} with network_egress=true + anthropic_api_key", secrets_path.display());
        }
        let raw = std::fs::read_to_string(secrets_path)?;
        let s: Secrets = toml::from_str(&raw)?;
        if !s.network_egress {
            anyhow::bail!("network_egress must be true to enable Anthropic codify provider");
        }
        let key = s.anthropic_api_key.ok_or_else(|| anyhow::anyhow!("missing anthropic_api_key"))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self { api_key: key, model, http })
    }
}

#[derive(Deserialize)]
struct Msg { content: Vec<MsgPart>, usage: Usage }
#[derive(Deserialize)]
struct MsgPart { text: Option<String> }
#[derive(Deserialize)]
struct Usage { input_tokens: u32, output_tokens: u32 }

#[async_trait]
impl CodifyProvider for AnthropicCodifyProvider {
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        use crate::services::codify::ollama::parse_decision;
        use crate::services::codify::prompts::SYSTEM_PROMPT;

        let user_prompt = format!(
            "Observation kind: {}\nFrequency: {}\nProject cwds: {:?}\n\nBundled context:\n---\n{}\n---\n\nReturn JSON now.",
            req.observation_kind, req.frequency, req.cwds, req.bundled_context,
        );
        let body = json!({
            "model": self.model,
            "system": SYSTEM_PROMPT,
            "messages": [{"role":"user","content": user_prompt}],
            "max_tokens": req.max_output_tokens as i32,
        });
        let resp = self.http.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body).send().await?
            .error_for_status()?;
        let m: Msg = resp.json().await?;
        let text = m.content.into_iter().find_map(|p| p.text).unwrap_or_default();
        let decision = parse_decision(&text)?;
        Ok(CodifyResult {
            decision,
            input_tokens: m.usage.input_tokens,
            output_tokens: m.usage.output_tokens,
        })
    }
    fn name(&self) -> &str { "anthropic" }
}
```

(`parse_decision` is the helper in `ollama.rs`. Make sure it's `pub(crate)` or `pub` so anthropic.rs can call it. Adjust visibility.)

- [ ] **Step 2: Register**

In `crates/teramindd/src/services/codify/mod.rs`, add `pub mod anthropic;`.

### Task 18.2: Wire into app.rs factory

**File:** Modify `crates/teramindd/src/app.rs`.

- [ ] **Step 1**

Replace the `_ => null` branch:

```rust
"anthropic" => {
    let secrets = paths.config_dir.join("secrets.toml");
    match crate::services::codify::anthropic::AnthropicCodifyProvider::try_new(&secrets, codify_cfg.model.clone()) {
        Ok(p) => std::sync::Arc::new(p),
        Err(e) => {
            tracing::warn!(error = %e, "Anthropic codify provider unavailable; falling back to null");
            std::sync::Arc::new(crate::services::codify::null::NullCodifyProvider)
        }
    }
}
_ => std::sync::Arc::new(crate::services::codify::null::NullCodifyProvider),
```

### Task 18.3: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramindd/src/services/codify/anthropic.rs \
        crates/teramindd/src/services/codify/ollama.rs \
        crates/teramindd/src/services/codify/mod.rs \
        crates/teramindd/src/app.rs
git commit -m "feat(codify): Anthropic provider (gated by network_egress)"
```

---

## Section 19 — Privacy test (DecisionCache filter)

### Task 19.1: Test

**File:** Create `crates/teramindd/tests/codify_privacy.rs`.

- [ ] **Step 1**

The detector privacy filter test. In local-first mode, with a `share = DeniedKeepLocal` decision for a particular session, that session should never appear in detector seed sets. This requires the detector to filter via `DecisionCache` if one is provided.

If the detectors currently take a `DecisionCache` parameter, extend the existing test scaffolding. If they don't (the implementation above shows detectors that don't filter via cache), **extend the detectors now**:

Add an optional `cache: Option<Arc<DecisionCache>>` parameter to each detector's `run` fn. When present, after the SQL fetch, filter session_ids to drop those whose decision is `DeniedKeepLocal`. Update callers in `codifier_worker.rs::detector_loop` to pass `Some(cache)`.

Test sketch:

```rust
//! With a session marked DeniedKeepLocal, detector A skips it.

use std::sync::Arc;
use teramind_core::ids::{SessionId, TurnId};
use teramind_db::repos::{AgentRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::tool_chain;
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn denied_sessions_excluded_from_observations() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();

    let mut sids = vec![];
    for i in 0..4 {
        let sid = sessions.insert(NewSession {
            agent_id: agent.id, agent_session_id: None, cwd: "/proj",
            project_id: None, parent_session_id: None,
            git_head: None, git_branch: None,
            os: "linux", hostname: "h", user_login: "u",
            started_at: started, user_id: None, device_id: None,
        }).await?;
        let tid = trace.upsert_turn_with_id(TurnId(Uuid::new_v4()), sid, 0, started, Some("x")).await?;
        trace.finalize_turn(tid, started, Some("y"), None, None, None, None).await?;
        trace.insert_tool_call(tid, 0, "Bash", &serde_json::json!({"command":"cargo build"}), Some("ok"), false, 100, started).await?;
        trace.insert_tool_call(tid, 1, "Bash", &serde_json::json!({"command":"cargo test"}), Some("ok"), false, 100, started).await?;
        sids.push(sid);
        // Deny the first session.
        if i == 0 {
            // Cache reference passed later.
        }
    }
    let cache = DecisionCache::new();
    cache.set_initial(sids[0], ShareDecision::DeniedKeepLocal);

    let obs = SkillObservationRepo::new(pool.clone());
    tool_chain::run_with_cache(&pool, &obs, time::Duration::days(30), Some(cache.clone())).await?;

    let above = obs.list_open(3, 10).await?;
    assert_eq!(above.len(), 1);
    assert_eq!(above[0].frequency, 3, "denied session must be excluded");
    let denied_uuid = sids[0].0;
    assert!(!above[0].session_ids.contains(&denied_uuid), "denied session id must not appear");

    sup.shutdown().await?;
    Ok(())
}
```

(`tool_chain::run_with_cache` is the parameterized version. Either rename existing `run` to take an optional cache, or add a new fn — pick whichever yields the smaller diff.)

### Task 19.2: Verify + commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramindd --test codify_privacy -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramindd/src/services/codify/detectors \
        crates/teramindd/src/services/codifier_worker.rs \
        crates/teramindd/tests/codify_privacy.rs
git commit -m "feat(codify): detectors honor DecisionCache (privacy filter)"
```

---

## Section 20 — Final check

### Task 20.1: Workspace sweep

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test --workspace --no-fail-fast -- --test-threads=1 2>&1 | grep -E "^test result" | tail
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plan L baseline: 330 tests. Plan M adds approximately:
- §1 migration: 1
- §2 SkillObservationRepo: 3
- §3 SkillCandidateRepo: 3
- §5 glob: 5
- §6 heuristics: 5
- §7 tool_chain detector: 1
- §8 problem_fix detector: 1
- §9 prompts: 1
- §10 Ollama parser: 3
- §11 llm_proposal detector: 1
- §13 codifier_worker_e2e: 1
- §15 MCP codify tool: 1
- §16 skills_cli: 1
- §19 privacy: 1

Total new: ~28 tests. Expected total: ~358.

### Task 20.2: Lint + fmt

If `cargo fmt --check` is dirty, run `cargo fmt --all` and commit as fresh `style: cargo fmt --all`.

### Task 20.3: Report

Print HEAD SHA, total commits from main (`git rev-list --count main..HEAD`), total tests, any failures. Do NOT push.

---

## Spec coverage matrix

| Spec section | Plan M addresses |
|---|---|
| §2.1 In-scope — two-stage pipeline | §6 (heuristics) + §7 (detector A) + §8 (detector B) + §11 (detector C) + §12 (synthesis) |
| §2.1 In-scope — three detectors | §7, §8, §11 |
| §2.1 In-scope — two entry points | §13 (worker autonomous loop) + §15 (MCP tool) |
| §2.1 In-scope — new storage | §1 (migration) + §2 (observation repo) + §3 (candidate repo) |
| §2.1 In-scope — CodifyProvider trait | §4 (trait) + §9 (Null) + §10 (Ollama) + §18 (Anthropic) |
| §2.1 In-scope — CLI surfaces | §16 |
| §2.1 In-scope — SessionStart digest | §17 |
| §2.1 In-scope — SQL admin approval + auto-promote | §12 (promote) + §13 (worker promote loop) |
| §2.1 In-scope — privacy | §19 (DecisionCache filter) |
| §2.1 In-scope — team mode | §14 (IPC dispatch reused by /v1/rpc via Plan K) |
| §2.1 In-scope — doctor surface | §17 |
| §3 Architecture | §13 + §17 (wiring) |
| §4 Storage | §1, §2, §3 |
| §5 Detectors | §6 (heuristics) + §7 + §8 + §11 |
| §6 Synthesis (bundler, trait, prompt, decision handling, promotion) | §4 + §9 + §10 + §12 + §13 |
| §7 MCP tool + CLI | §15 + §16 |
| §8 Auto-load digest | §17 |
| §9 Configuration | §9 (CodifyConfig) + §17 (doctor) + §18 (Anthropic gating) |
| §10 Testing strategy | tests across §1-§19 |
| §11 Rollout, risks | implicitly covered; no code |
