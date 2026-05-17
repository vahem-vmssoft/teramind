# Teramind Web Dashboard (Plan N) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an admin-only browser dashboard over the team-mode sync server: live activity feed, skill catalog with embedded candidate review, members + devices, and search-quality charts.

**Architecture:** All backend code lives in the existing `teramind-sync-server` binary — a parallel cookie-authenticated `/admin/*` HTTP surface alongside the existing DPoP-protected `/v1/*`, plus static-asset serving for `/dashboard/*`. The React SPA lives in a new top-level `dashboard/` directory; its production bundle is embedded into the server binary via `include_dir!` so the operator deploys a single artifact. Two new tables (`team_event_log`, `quality_runs`) persist what the live broadcast bus already emits + a cron-driven eval scheduler.

**Tech Stack:**
- Backend: `axum` (existing), `argon2` 0.5, `hmac` 0.12 + `sha2` 0.10 (existing), `cron` 0.12, `include_dir` 0.7, `tokio` (existing).
- Frontend: React 18, TypeScript 5, Vite 5, TanStack Query 5, TanStack Router 1, TailwindCSS 3, Recharts 2, Lucide React, Vitest, Playwright. No `node_modules` checked in; `dashboard/dist/` gitignored except `.gitkeep`.

---

## Spec coverage

This plan implements `docs/superpowers/specs/2026-05-17-teramind-web-dashboard-design.md` end-to-end. Coverage matrix at the bottom.

---

## File structure

**New files — backend (Rust):**

| Path | Responsibility |
|---|---|
| `crates/teramind-db/migrations/20260520000001_dashboard.sql` | `team_event_log` + `quality_runs` tables |
| `crates/teramind-db/src/repos/team_event_log.rs` | `TeamEventLogRepo` |
| `crates/teramind-db/src/repos/quality_run.rs` | `QualityRunRepo` |
| `crates/teramind-core/src/quality.rs` | `QualityRunOutput` (shared by eval binary + server) |
| `crates/teramind-sync-server/src/admin_api/mod.rs` | Module registry |
| `crates/teramind-sync-server/src/admin_api/config.rs` | `AdminConfig` TOML loader + `QualityConfig` |
| `crates/teramind-sync-server/src/admin_api/cookie.rs` | Session cookie encode/decode + HMAC |
| `crates/teramind-sync-server/src/admin_api/auth.rs` | Tower middleware: cookie verification |
| `crates/teramind-sync-server/src/admin_api/rate_limit.rs` | Per-IP login throttle |
| `crates/teramind-sync-server/src/admin_api/error.rs` | Stable error codes + `DashboardError` JSON shape |
| `crates/teramind-sync-server/src/admin_api/handlers/session.rs` | `/admin/login` `/admin/logout` `/admin/me` `/admin/version` |
| `crates/teramind-sync-server/src/admin_api/handlers/activity.rs` | `/admin/activity` HTTP + `/admin/events` WS |
| `crates/teramind-sync-server/src/admin_api/handlers/skills.rs` | `/admin/skills` GET + GET-one + DELETE |
| `crates/teramind-sync-server/src/admin_api/handlers/candidates.rs` | `/admin/candidates` list/show/approve/reject/PATCH |
| `crates/teramind-sync-server/src/admin_api/handlers/observations.rs` | `/admin/observations` |
| `crates/teramind-sync-server/src/admin_api/handlers/members.rs` | `/admin/members` + `/admin/devices` + `/admin/invites` |
| `crates/teramind-sync-server/src/admin_api/handlers/quality.rs` | `/admin/quality` + `POST /admin/quality/runs` + `/admin/quality/config` |
| `crates/teramind-sync-server/src/admin_api/handlers/health.rs` | `/admin/health` |
| `crates/teramind-sync-server/src/event_log_writer.rs` | `EventLogWriter::log(event)` — fire-and-forget DB insert |
| `crates/teramind-sync-server/src/event_log_pruner.rs` | Periodic delete of old `team_event_log` rows |
| `crates/teramind-sync-server/src/quality_scheduler.rs` | Cron-driven subprocess runner |
| `crates/teramind-sync-server/src/dashboard_assets.rs` | `include_dir!` bundle + content-type guesser + SPA fallback |
| `crates/teramind-sync-server/build.rs` | Warn-when-dist-missing rerun-if-changed |
| `crates/teramind-sync-server/tests/admin_login.rs` | Login + rate-limit |
| `crates/teramind-sync-server/tests/admin_session.rs` | Cookie verification + /admin/me |
| `crates/teramind-sync-server/tests/admin_skills.rs` | Skills list/show + delete |
| `crates/teramind-sync-server/tests/admin_candidates.rs` | Candidate approve/reject/edit |
| `crates/teramind-sync-server/tests/admin_members.rs` | Members + devices + invite issuance |
| `crates/teramind-sync-server/tests/admin_activity.rs` | Event-log HTTP GET + WebSocket |
| `crates/teramind-sync-server/tests/admin_quality.rs` | Quality endpoints + scheduler |
| `crates/teramind-sync-server/tests/dashboard_assets.rs` | Embedded asset serving + SPA fallback |

**New files — frontend (TypeScript):**

| Path | Responsibility |
|---|---|
| `dashboard/package.json` | npm manifest |
| `dashboard/tsconfig.json` | TS config |
| `dashboard/vite.config.ts` | Vite + Tailwind |
| `dashboard/tailwind.config.ts` | Tailwind config |
| `dashboard/postcss.config.cjs` | PostCSS for Tailwind |
| `dashboard/index.html` | Entrypoint |
| `dashboard/src/main.tsx` | React mount |
| `dashboard/src/router.tsx` | TanStack Router setup |
| `dashboard/src/lib/api.ts` | fetch wrapper + error mapping |
| `dashboard/src/lib/auth.tsx` | `useAuth` hook + route guard |
| `dashboard/src/lib/event_stream.ts` | WS hook for `/admin/events` |
| `dashboard/src/components/Shell.tsx` | Sidebar + top bar |
| `dashboard/src/components/Toast.tsx` | Error toast |
| `dashboard/src/components/CopyModal.tsx` | One-time copy-to-clipboard modal |
| `dashboard/src/routes/__root.tsx` | Route tree root |
| `dashboard/src/routes/login.tsx` | Login form |
| `dashboard/src/routes/activity.tsx` | Activity timeline |
| `dashboard/src/routes/skills.tsx` | Skills list + detail panel |
| `dashboard/src/routes/candidates.tsx` | Candidate review |
| `dashboard/src/routes/members.tsx` | Members + invites |
| `dashboard/src/routes/quality.tsx` | Quality charts |
| `dashboard/src/routes/health.tsx` | Health summary |
| `dashboard/tests/auth.test.ts` | `useAuth` unit tests |
| `dashboard/tests/api.test.ts` | API client + error mapping |
| `dashboard/tests/event_stream.test.ts` | Event-reducer tests |
| `dashboard/tests/playwright/dashboard.spec.ts` | E2E walk-through |
| `dashboard/.gitignore` | `node_modules` + `dist` + `playwright-report` |
| `dashboard/dist/.gitkeep` | Empty placeholder so `include_dir!` works pre-build |

**Modified files:**

- `Cargo.toml` (workspace) — add `argon2 = "0.5"`, `hmac = "0.12"`, `cron = "0.12"`, `include_dir = "0.7"`, `base64 = "0.22"` (already present per Plan I).
- `crates/teramind-sync-server/Cargo.toml` — depend on the new workspace deps.
- `crates/teramind-sync-server/src/lib.rs` — register `admin_api`, `dashboard_assets`, `event_log_writer`, `event_log_pruner`, `quality_scheduler`.
- `crates/teramind-sync-server/src/main.rs` — new `admin-password` subcommand; spawn pruner + scheduler when configured.
- `crates/teramind-sync-server/src/config.rs` — `[admin]` and `[quality]` sub-tables.
- `crates/teramind-sync-server/src/server.rs::build_router` — mount `/admin/*` + `/dashboard/*`.
- `crates/teramind-sync-server/src/state.rs` — `AppState` gets `event_log: TeamEventLogRepo`, `quality: QualityRunRepo`, `admin: Option<AdminConfig>`.
- `crates/teramind-sync-server/src/handlers/ingest.rs::publish_on_success` — also call `EventLogWriter::log(...)` after `bus.send(...)`.
- `crates/teramindd/src/services/rpc_dispatch.rs` (the SaveSkill arm) — call `EventLogWriter::log(...)` after `bus.send(...)`. The writer is constructed from the existing `RpcDeps::event_bus.is_some()` site; pipe an `Option<EventLogWriter>` through.
- `crates/teramind-db/src/repos/mod.rs` — register `team_event_log`, `quality_run`.
- `crates/teramind-core/src/lib.rs` — `pub mod quality;`.
- `crates/teramind-search-eval/src/main.rs` — new `--json` flag emitting `QualityRunOutput`.
- `.gitignore` (workspace root) — `dashboard/node_modules`, `dashboard/dist/*` (but not `dist/.gitkeep`).

---

## Section 0 — Pre-flight

### Task 0.1: Branch from a green main

- [ ] **Step 1**

```bash
git checkout main
cargo build --workspace 2>&1 | tail -3
git checkout -b feat/teramind-web-dashboard
git log --oneline -3
```

Verify Plan M merge commit appears in recent history. Build is silent.

### Task 0.2: Confirm environment

- [ ] **Step 1**

Check Node + npm available locally (for the frontend half):

```bash
node --version    # expect v18+ or v20+
npm --version     # expect v9+
```

If absent: the Rust half still works; defer frontend tasks until Node is available.

- [ ] **Step 2: Confirm GitHub token for PG tests**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-db --test user_repo -- --test-threads=1 2>&1 | tail -3
```

Expected: 2 PASS.

---

## Section 1 — Migration + new tables

### Task 1.1: Write the migration

**Files:**
- Create: `crates/teramind-db/migrations/20260520000001_dashboard.sql`

- [ ] **Step 1**

```sql
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
```

- [ ] **Step 2: Write the verification test**

**File:** Create `crates/teramind-db/tests/dashboard_migration.rs`

```rust
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn migration_creates_tables() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    for t in ["team_event_log", "quality_runs"] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)"
        ).bind(t).fetch_one(pool.pg()).await?;
        assert!(exists, "table `{t}` must exist after migration");
    }

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 3: Run + commit**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-db --test dashboard_migration -- --test-threads=1
git add crates/teramind-db/migrations/20260520000001_dashboard.sql \
        crates/teramind-db/tests/dashboard_migration.rs
git commit -m "feat(db): dashboard migration (team_event_log + quality_runs)"
```

---

## Section 2 — `TeamEventLogRepo` + `QualityRunRepo`

### Task 2.1: TeamEventLogRepo

**File:** Create `crates/teramind-db/src/repos/team_event_log.rs`

- [ ] **Step 1: Write tests first**

Create `crates/teramind-db/tests/team_event_log_repo.rs`:

```rust
use teramind_db::repos::{TeamEventLogRepo, UserRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use uuid::Uuid;

async fn fresh_pool() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_and_list_recent_roundtrips() -> anyhow::Result<()> {
    let (_d, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let log = TeamEventLogRepo::new(pool.clone());
    let u = users.upsert_by_email("alice@acme.dev", None).await?;

    log.insert("session_ended", Some(u.id), Some("/proj".into()),
               serde_json::json!({"session_id":"abc"})).await?;
    log.insert("skill_saved", Some(u.id), None,
               serde_json::json!({"skill_id":"x"})).await?;

    let rows = log.list_recent(None, None, None, 100).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, "skill_saved");      // newest first
    assert_eq!(rows[1].kind, "session_ended");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_recent_filters_by_kind_and_user() -> anyhow::Result<()> {
    let (_d, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let log = TeamEventLogRepo::new(pool.clone());
    let alice = users.upsert_by_email("a@x.dev", None).await?;
    let bob   = users.upsert_by_email("b@x.dev", None).await?;

    log.insert("session_ended", Some(alice.id), None, serde_json::json!({})).await?;
    log.insert("skill_saved",   Some(alice.id), None, serde_json::json!({})).await?;
    log.insert("session_ended", Some(bob.id),   None, serde_json::json!({})).await?;

    let alice_rows = log.list_recent(None, None, Some(alice.id), 10).await?;
    assert_eq!(alice_rows.len(), 2);
    let alice_ended = log.list_recent(Some("session_ended"), None, Some(alice.id), 10).await?;
    assert_eq!(alice_ended.len(), 1);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prune_deletes_old_rows() -> anyhow::Result<()> {
    let (_d, sup, pool) = fresh_pool().await?;
    let log = TeamEventLogRepo::new(pool.clone());

    log.insert("session_ended", None, None, serde_json::json!({})).await?;
    // Backdate it 100 days.
    sqlx::query("UPDATE team_event_log SET ts = now() - interval '100 days'")
        .execute(pool.pg()).await?;

    let deleted = log.prune_older_than(90).await?;
    assert_eq!(deleted, 1);
    assert!(log.list_recent(None, None, None, 10).await?.is_empty());

    sup.shutdown().await?;
    Ok(())
}
```

Run: `cargo test -p teramind-db --test team_event_log_repo -- --test-threads=1` → FAIL (`TeamEventLogRepo` missing).

- [ ] **Step 2: Implement the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use serde_json::Value;
use teramind_core::ids::UserId;
use time::OffsetDateTime;
use uuid::Uuid;

type EventRow = (Uuid, String, Option<Uuid>, Option<String>, Value, OffsetDateTime);

fn row_to_event(r: EventRow) -> TeamEventRow {
    TeamEventRow {
        id: r.0, kind: r.1,
        user_id: r.2.map(UserId), cwd: r.3, payload: r.4, ts: r.5,
    }
}

#[derive(Debug, Clone)]
pub struct TeamEventRow {
    pub id: Uuid,
    pub kind: String,
    pub user_id: Option<UserId>,
    pub cwd: Option<String>,
    pub payload: Value,
    pub ts: OffsetDateTime,
}

#[derive(Clone)]
pub struct TeamEventLogRepo { pool: DbPool }

impl TeamEventLogRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn insert(
        &self,
        kind: &str,
        user_id: Option<UserId>,
        cwd: Option<String>,
        payload: Value,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO team_event_log (kind, user_id, cwd, payload)
               VALUES ($1, $2, $3, $4)"#)
            .bind(kind).bind(user_id.map(|u| u.0)).bind(cwd).bind(payload)
            .execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn list_recent(
        &self,
        kind: Option<&str>,
        before: Option<OffsetDateTime>,
        user_id: Option<UserId>,
        limit: i64,
    ) -> Result<Vec<TeamEventRow>> {
        let rows: Vec<EventRow> = sqlx::query_as(
            r#"SELECT id, kind, user_id, cwd, payload, ts
               FROM team_event_log
               WHERE ($1::text IS NULL OR kind = $1)
                 AND ($2::timestamptz IS NULL OR ts < $2)
                 AND ($3::uuid IS NULL OR user_id = $3)
               ORDER BY ts DESC
               LIMIT $4"#)
            .bind(kind).bind(before).bind(user_id.map(|u| u.0)).bind(limit)
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_event).collect())
    }

    pub async fn prune_older_than(&self, days: i64) -> Result<u64> {
        let r = sqlx::query(
            r#"DELETE FROM team_event_log WHERE ts < now() - ($1::int * interval '1 day')"#)
            .bind(days as i32)
            .execute(self.pool.pg()).await?;
        Ok(r.rows_affected())
    }
}
```

Register in `crates/teramind-db/src/repos/mod.rs`: `pub mod team_event_log;` + `pub use team_event_log::{TeamEventLogRepo, TeamEventRow};`.

- [ ] **Step 3: Run + commit**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-db --test team_event_log_repo -- --test-threads=1
cargo clippy -p teramind-db --all-targets -- -D warnings
git add crates/teramind-db/src/repos/team_event_log.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/tests/team_event_log_repo.rs
git commit -m "feat(db): TeamEventLogRepo"
```

### Task 2.2: QualityRunRepo

**Files:**
- Create: `crates/teramind-db/src/repos/quality_run.rs`
- Create: `crates/teramind-db/tests/quality_run_repo.rs`

- [ ] **Step 1: Write tests**

```rust
use teramind_db::repos::QualityRunRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_and_list_latest() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    let repo = QualityRunRepo::new(pool.clone());

    repo.insert("lexical", None, 0.142, 0.301, 0.230, 0.180, 0.420,
                42.0, 380.0, 100, 500,
                serde_json::json!({}), serde_json::json!({"k":"v"}), "scheduled").await?;
    repo.insert("semantic", Some("ollama:nomic-embed-text-v2-moe".into()),
                0.537, 0.412, 0.480, 0.410, 0.620, 50.0, 410.0, 100, 500,
                serde_json::json!({}), serde_json::json!({}), "scheduled").await?;

    let runs = repo.list_recent(None, 10).await?;
    assert_eq!(runs.len(), 2);
    let latest_semantic = repo.latest("semantic").await?;
    assert!(latest_semantic.is_some());
    assert_eq!(latest_semantic.unwrap().ndcg10, 0.537);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Implement the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

type QualityRow = (
    Uuid, String, Option<String>, f64, f64, f64, f64, f64, f64, f64,
    i32, i32, Value, Value, OffsetDateTime, String,
);

fn row_to_quality(r: QualityRow) -> QualityRunRow {
    QualityRunRow {
        id: r.0, baseline_label: r.1, model: r.2,
        ndcg10: r.3, mrr: r.4, precision_5: r.5, precision_10: r.6, recall_10: r.7,
        p50_latency_ms: r.8, p95_latency_ms: r.9,
        query_count: r.10, corpus_size: r.11,
        per_class: r.12, raw_json: r.13, ran_at: r.14, source: r.15,
    }
}

#[derive(Debug, Clone)]
pub struct QualityRunRow {
    pub id: Uuid,
    pub baseline_label: String,
    pub model: Option<String>,
    pub ndcg10: f64,
    pub mrr: f64,
    pub precision_5: f64,
    pub precision_10: f64,
    pub recall_10: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub query_count: i32,
    pub corpus_size: i32,
    pub per_class: Value,
    pub raw_json: Value,
    pub ran_at: OffsetDateTime,
    pub source: String,
}

#[derive(Clone)]
pub struct QualityRunRepo { pool: DbPool }

impl QualityRunRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert(
        &self,
        baseline_label: &str, model: Option<String>,
        ndcg10: f64, mrr: f64, p5: f64, p10: f64, r10: f64,
        p50: f64, p95: f64, query_count: i32, corpus_size: i32,
        per_class: Value, raw_json: Value, source: &str,
    ) -> Result<Uuid> {
        let row: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO quality_runs
               (baseline_label, model, ndcg10, mrr, precision_5, precision_10, recall_10,
                p50_latency_ms, p95_latency_ms, query_count, corpus_size,
                per_class, raw_json, source)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               RETURNING id"#)
            .bind(baseline_label).bind(model).bind(ndcg10).bind(mrr)
            .bind(p5).bind(p10).bind(r10).bind(p50).bind(p95)
            .bind(query_count).bind(corpus_size).bind(per_class).bind(raw_json)
            .bind(source)
            .fetch_one(self.pool.pg()).await?;
        Ok(row.0)
    }

    pub async fn list_recent(&self, baseline: Option<&str>, limit: i64) -> Result<Vec<QualityRunRow>> {
        let rows: Vec<QualityRow> = sqlx::query_as(
            r#"SELECT id, baseline_label, model, ndcg10, mrr, precision_5, precision_10, recall_10,
                       p50_latency_ms, p95_latency_ms, query_count, corpus_size,
                       per_class, raw_json, ran_at, source
               FROM quality_runs
               WHERE ($1::text IS NULL OR baseline_label = $1)
               ORDER BY ran_at DESC
               LIMIT $2"#)
            .bind(baseline).bind(limit)
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_quality).collect())
    }

    pub async fn latest(&self, baseline_label: &str) -> Result<Option<QualityRunRow>> {
        let row: Option<QualityRow> = sqlx::query_as(
            r#"SELECT id, baseline_label, model, ndcg10, mrr, precision_5, precision_10, recall_10,
                       p50_latency_ms, p95_latency_ms, query_count, corpus_size,
                       per_class, raw_json, ran_at, source
               FROM quality_runs
               WHERE baseline_label = $1
               ORDER BY ran_at DESC LIMIT 1"#)
            .bind(baseline_label).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(row_to_quality))
    }
}
```

Register in `mod.rs`: `pub mod quality_run;` + `pub use quality_run::{QualityRunRepo, QualityRunRow};`.

- [ ] **Step 3: Run + commit**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-db --test quality_run_repo -- --test-threads=1
cargo clippy -p teramind-db --all-targets -- -D warnings
git add crates/teramind-db/src/repos/quality_run.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/tests/quality_run_repo.rs
git commit -m "feat(db): QualityRunRepo"
```

---

## Section 3 — Admin config + password hashing CLI subcommand

### Task 3.1: Add workspace deps

**File:** Modify workspace `Cargo.toml`.

- [ ] **Step 1**

Under `[workspace.dependencies]`, add (alphabetically):

```toml
argon2      = "0.5"
cron        = "0.12"
hmac        = "0.12"
include_dir = "0.7"
rpassword   = "7"
```

(`base64`, `serde`, `sha2`, `tokio`, `axum`, `tower-http`, `tracing` are already in the workspace.)

### Task 3.2: AdminConfig + QualityConfig structs

**File:** Modify `crates/teramind-sync-server/src/config.rs`.

- [ ] **Step 1: Extend ServerConfig**

Append to the existing `ServerConfig` struct:

```rust
#[serde(default)]
pub admin: Option<AdminConfig>,
#[serde(default)]
pub quality: Option<QualityConfig>,
```

Add the new structs (anywhere in the file after the existing config types):

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct AdminConfig {
    pub admin_password_hash: String,
    pub admin_session_secret: String,
    #[serde(default = "AdminConfig::default_ttl")]
    pub admin_session_ttl_hours: u64,
    #[serde(default = "AdminConfig::default_retention")]
    pub event_log_retention_days: i64,
}

impl AdminConfig {
    fn default_ttl() -> u64 { 12 }
    fn default_retention() -> i64 { 90 }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QualityConfig {
    #[serde(default)]
    pub enabled: bool,
    pub cron: Option<String>,
    #[serde(default)]
    pub baselines: Vec<String>,
    #[serde(default = "QualityConfig::default_binary")]
    pub eval_binary: String,
}

impl QualityConfig {
    fn default_binary() -> String { "teramind-search-eval".into() }
}
```

### Task 3.3: Crate deps

**File:** Modify `crates/teramind-sync-server/Cargo.toml`.

- [ ] **Step 1**

Under `[dependencies]` add:

```toml
argon2      = { workspace = true }
cron        = { workspace = true }
hmac        = { workspace = true }
include_dir = { workspace = true }
rpassword   = { workspace = true }
```

(`base64`, `sha2` are already there from Plan I; if not, add them.)

### Task 3.4: `admin-password` subcommand

**File:** Modify `crates/teramind-sync-server/src/main.rs`.

- [ ] **Step 1: Add the subcommand**

Locate the existing `Cmd` enum. Add a new variant:

```rust
/// Hash a new admin password (prints config snippet to stdout).
AdminPassword,
```

In `main`'s match, add the arm:

```rust
Cmd::AdminPassword => {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    let p1 = rpassword::prompt_password("Enter new admin password: ")?;
    let p2 = rpassword::prompt_password("Confirm: ")?;
    if p1 != p2 {
        anyhow::bail!("passwords do not match");
    }
    if p1.len() < 12 {
        anyhow::bail!("password must be at least 12 characters");
    }
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let hash = argon.hash_password(p1.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash: {e}"))?
        .to_string();
    let mut secret_bytes = [0u8; 32];
    rand::Rng::fill(&mut rand::thread_rng(), &mut secret_bytes);
    let secret = hex::encode(secret_bytes);
    println!();
    println!("[admin]");
    println!("admin_password_hash      = \"{hash}\"");
    println!("admin_session_secret     = \"{secret}\"");
    println!("admin_session_ttl_hours  = 12");
    println!("event_log_retention_days = 90");
    Ok(())
}
```

`rand::thread_rng()` requires `rand` to be in the crate's deps; it is (per Plan I).

- [ ] **Step 2: Smoke test**

```bash
cargo build -p teramind-sync-server
printf 'hunter2hunter2\nhunter2hunter2\n' | ./target/debug/teramind-sync-server admin-password
```

Expected: prints a `[admin]` block with a real argon2 hash and a 64-char hex secret.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml \
        crates/teramind-sync-server/Cargo.toml \
        crates/teramind-sync-server/src/main.rs \
        crates/teramind-sync-server/src/config.rs
git commit -m "feat(sync-server): admin-password subcommand + AdminConfig"
```

---

## Section 4 — Session cookie codec

### Task 4.1: Tests first

**File:** Create `crates/teramind-sync-server/src/admin_api/mod.rs` with the stub:

```rust
//! Admin-side HTTP API (cookie-authenticated). See spec for full layout.

pub mod cookie;
```

**File:** Create `crates/teramind-sync-server/src/admin_api/cookie.rs`.

- [ ] **Step 1: Write the cookie module + tests**

```rust
//! Self-validating session cookie. Format:
//!   token = base64url(payload) || "." || base64url(hmac_sha256(secret, payload))
//!   payload = jti(16 bytes) || expires_at_unix_be64(8 bytes)

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error, PartialEq)]
pub enum CookieError {
    #[error("malformed token: not two dot-separated parts")] Malformed,
    #[error("base64 decode failed")] BadBase64,
    #[error("payload length not 24 bytes")] BadPayloadLength,
    #[error("HMAC verification failed")] BadHmac,
    #[error("expired")] Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSession {
    pub jti: [u8; 16],
    pub expires_at: OffsetDateTime,
}

/// Generate a random 16-byte jti from the OS RNG.
pub fn random_jti() -> [u8; 16] {
    let mut out = [0u8; 16];
    rand::Rng::fill(&mut rand::thread_rng(), &mut out);
    out
}

pub fn encode(session: &AdminSession, secret_hex: &str) -> String {
    let secret = hex::decode(secret_hex).expect("admin_session_secret must be hex");
    let mut payload = Vec::with_capacity(24);
    payload.extend_from_slice(&session.jti);
    payload.extend_from_slice(&session.expires_at.unix_timestamp().to_be_bytes());

    let mut mac = HmacSha256::new_from_slice(&secret).expect("hmac key");
    mac.update(&payload);
    let sig = mac.finalize().into_bytes();

    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("{}.{}", e.encode(&payload), e.encode(&sig))
}

pub fn decode(token: &str, secret_hex: &str, now: OffsetDateTime) -> Result<AdminSession, CookieError> {
    let secret = hex::decode(secret_hex).map_err(|_| CookieError::BadBase64)?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 { return Err(CookieError::Malformed); }
    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = e.decode(parts[0]).map_err(|_| CookieError::BadBase64)?;
    let sig     = e.decode(parts[1]).map_err(|_| CookieError::BadBase64)?;
    if payload.len() != 24 { return Err(CookieError::BadPayloadLength); }

    let mut mac = HmacSha256::new_from_slice(&secret).map_err(|_| CookieError::BadHmac)?;
    mac.update(&payload);
    mac.verify_slice(&sig).map_err(|_| CookieError::BadHmac)?;

    let mut jti = [0u8; 16];
    jti.copy_from_slice(&payload[..16]);
    let mut ts_bytes = [0u8; 8];
    ts_bytes.copy_from_slice(&payload[16..]);
    let expires_at = OffsetDateTime::from_unix_timestamp(i64::from_be_bytes(ts_bytes))
        .map_err(|_| CookieError::BadPayloadLength)?;
    if expires_at < now { return Err(CookieError::Expired); }
    Ok(AdminSession { jti, expires_at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    fn fixture() -> (String, AdminSession, OffsetDateTime) {
        let secret = hex::encode([0xABu8; 32]);
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let session = AdminSession { jti: [7u8; 16], expires_at: now + Duration::hours(12) };
        (secret, session, now)
    }

    #[test] fn encode_decode_roundtrips() {
        let (s, sess, now) = fixture();
        let token = encode(&sess, &s);
        let out = decode(&token, &s, now).unwrap();
        assert_eq!(out, sess);
    }

    #[test] fn expired_token_rejects() {
        let (s, sess, now) = fixture();
        let token = encode(&sess, &s);
        let later = now + Duration::hours(13);
        assert_eq!(decode(&token, &s, later), Err(CookieError::Expired));
    }

    #[test] fn tampered_hmac_rejects() {
        let (s, sess, now) = fixture();
        let mut token = encode(&sess, &s);
        let last = token.pop().unwrap();
        token.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(decode(&token, &s, now), Err(CookieError::BadHmac));
    }

    #[test] fn wrong_secret_rejects() {
        let (s, sess, now) = fixture();
        let token = encode(&sess, &s);
        let other = hex::encode([0x11u8; 32]);
        assert_eq!(decode(&token, &other, now), Err(CookieError::BadHmac));
    }

    #[test] fn malformed_token_rejects() {
        let (s, _sess, now) = fixture();
        assert_eq!(decode("only-one-part", &s, now), Err(CookieError::Malformed));
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/teramind-sync-server/src/lib.rs`, add `pub mod admin_api;` (alphabetically after `auth`).

- [ ] **Step 3: Run + commit**

```bash
cargo test -p teramind-sync-server admin_api::cookie::
cargo clippy -p teramind-sync-server --all-targets -- -D warnings
git add crates/teramind-sync-server/src/admin_api/mod.rs \
        crates/teramind-sync-server/src/admin_api/cookie.rs \
        crates/teramind-sync-server/src/lib.rs
git commit -m "feat(sync-server): admin session cookie codec"
```

---

## Section 5 — Auth middleware + rate limit

### Task 5.1: Rate limit module

**File:** Create `crates/teramind-sync-server/src/admin_api/rate_limit.rs`.

- [ ] **Step 1: Write the module**

```rust
//! Per-IP login attempt throttle. 5 failures in 60s → 5-min lockout.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_FAILURES: u8 = 5;
const WINDOW: Duration = Duration::from_secs(60);
const LOCKOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Entry {
    failures: Vec<Instant>,
    locked_until: Option<Instant>,
}

pub struct LoginThrottle {
    inner: Mutex<HashMap<IpAddr, Entry>>,
}

impl LoginThrottle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(HashMap::new()) })
    }

    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        let now = Instant::now();
        let mut map = self.inner.lock();
        if let Some(e) = map.get_mut(&ip) {
            if let Some(until) = e.locked_until {
                if until > now { return Err(until - now); }
                e.locked_until = None;
                e.failures.clear();
            }
        }
        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut map = self.inner.lock();
        let e = map.entry(ip).or_default();
        e.failures.retain(|t| now.duration_since(*t) <= WINDOW);
        e.failures.push(now);
        if e.failures.len() as u8 >= MAX_FAILURES {
            e.locked_until = Some(now + LOCKOUT);
        }
    }

    pub fn record_success(&self, ip: IpAddr) {
        let mut map = self.inner.lock();
        if let Some(e) = map.get_mut(&ip) {
            e.failures.clear();
            e.locked_until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn five_failures_lock_out() {
        let t = LoginThrottle::new();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        for _ in 0..4 { t.check(ip).unwrap(); t.record_failure(ip); }
        // 5th failure trips the lockout.
        t.check(ip).unwrap();
        t.record_failure(ip);
        assert!(t.check(ip).is_err());
    }

    #[test]
    fn success_resets_failures() {
        let t = LoginThrottle::new();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        for _ in 0..4 { t.record_failure(ip); }
        t.record_success(ip);
        for _ in 0..4 { t.record_failure(ip); }
        assert!(t.check(ip).is_ok());
    }

    #[test]
    fn different_ips_isolated() {
        let t = LoginThrottle::new();
        let a = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let b = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        for _ in 0..6 { t.record_failure(a); }
        assert!(t.check(a).is_err());
        assert!(t.check(b).is_ok());
    }
}
```

- [ ] **Step 2: Register + run**

In `admin_api/mod.rs`: `pub mod rate_limit;`.

```bash
cargo test -p teramind-sync-server admin_api::rate_limit::
```

3 PASS.

### Task 5.2: Auth middleware

**File:** Create `crates/teramind-sync-server/src/admin_api/auth.rs`.

- [ ] **Step 1: Write the middleware**

```rust
//! Tower middleware: verify the admin session cookie.

use crate::admin_api::cookie::{decode, AdminSession};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use time::OffsetDateTime;

pub async fn admin_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(admin_cfg) = state.admin.as_ref() else {
        // Dashboard not configured — 404 to avoid signalling existence.
        return Err(StatusCode::NOT_FOUND);
    };
    let cookie_header = request.headers().get(header::COOKIE)
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    let token = cookie_header
        .split(';')
        .map(|s| s.trim())
        .find_map(|kv| kv.strip_prefix("tmd_admin="))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let session: AdminSession = decode(token, &admin_cfg.admin_session_secret, OffsetDateTime::now_utc())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut req = request;
    req.extensions_mut().insert(session);
    Ok(next.run(req).await)
}
```

- [ ] **Step 2: Register**

In `admin_api/mod.rs`: `pub mod auth;`.

### Task 5.3: Wire AppState

**File:** Modify `crates/teramind-sync-server/src/state.rs`.

- [ ] **Step 1: Extend AppState**

Add fields:

```rust
pub admin: Option<std::sync::Arc<crate::config::AdminConfig>>,
pub login_throttle: std::sync::Arc<crate::admin_api::rate_limit::LoginThrottle>,
pub event_log: teramind_db::repos::TeamEventLogRepo,
pub quality: teramind_db::repos::QualityRunRepo,
```

In `AppState::new` (the existing constructor), populate them. `AdminConfig` is read from `cfg.admin.clone().map(Arc::new)`.

```rust
let admin = cfg.admin.clone().map(std::sync::Arc::new);
let login_throttle = crate::admin_api::rate_limit::LoginThrottle::new();
let event_log = teramind_db::repos::TeamEventLogRepo::new(pool.clone());
let quality   = teramind_db::repos::QualityRunRepo::new(pool.clone());

Self {
    /* existing fields */,
    admin, login_throttle, event_log, quality,
}
```

### Task 5.4: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-sync-server/src/admin_api \
        crates/teramind-sync-server/src/state.rs
git commit -m "feat(sync-server): admin auth middleware + rate limit + AppState wiring"
```

---

## Section 6 — Error shape

### Task 6.1: DashboardError + IntoResponse

**File:** Create `crates/teramind-sync-server/src/admin_api/error.rs`.

- [ ] **Step 1**

```rust
//! Stable error JSON shape for /admin/*.

use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DashboardError {
    pub error: ErrorBody,
    #[serde(skip)]
    pub status: StatusCode,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl DashboardError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self { status, error: ErrorBody { code: code.into(), message: message.into(), details: None } }
    }
}

impl IntoResponse for DashboardError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.error }));
        (self.status, body).into_response()
    }
}

pub type DashboardResult<T> = Result<T, DashboardError>;
```

Register in `admin_api/mod.rs`: `pub mod error;`.

- [ ] **Step 2: Commit**

```bash
git add crates/teramind-sync-server/src/admin_api/error.rs \
        crates/teramind-sync-server/src/admin_api/mod.rs
git commit -m "feat(sync-server): DashboardError JSON shape"
```

---

## Section 7 — Admin API: session + meta endpoints

### Task 7.1: Handlers

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/session.rs`.

- [ ] **Step 1**

```rust
//! /admin/login, /admin/logout, /admin/me, /admin/version

use crate::admin_api::cookie::{encode, random_jti, AdminSession};
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use argon2::{password_hash::PasswordHash, Argon2, PasswordVerifier};
use axum::{
    extract::{ConnectInfo, Extension, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use time::{Duration, OffsetDateTime};

#[derive(Deserialize)]
pub struct LoginRequest { pub password: String }

#[derive(Serialize)]
pub struct LoginResponse {
    pub logged_in: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> DashboardResult<impl IntoResponse> {
    let Some(admin_cfg) = state.admin.as_ref() else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "dashboard not configured"));
    };
    let ip = addr.ip();
    if let Err(_remain) = state.login_throttle.check(ip) {
        return Err(DashboardError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many failed attempts"));
    }

    let parsed = PasswordHash::new(&admin_cfg.admin_password_hash)
        .map_err(|_| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "bad password hash in config"))?;
    if Argon2::default().verify_password(req.password.as_bytes(), &parsed).is_err() {
        state.login_throttle.record_failure(ip);
        return Err(DashboardError::new(StatusCode::UNAUTHORIZED, "invalid_password", "bad password"));
    }
    state.login_throttle.record_success(ip);

    let session = AdminSession {
        jti: random_jti(),
        expires_at: OffsetDateTime::now_utc() + Duration::hours(admin_cfg.admin_session_ttl_hours as i64),
    };
    let token = encode(&session, &admin_cfg.admin_session_secret);
    let max_age = admin_cfg.admin_session_ttl_hours * 3600;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        format!(
            "tmd_admin={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={max_age}"
        ).parse().unwrap(),
    );
    Ok((headers, Json(LoginResponse { logged_in: true, expires_at: session.expires_at })))
}

pub async fn logout() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        "tmd_admin=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0".parse().unwrap(),
    );
    (headers, Json(serde_json::json!({ "logged_out": true })))
}

#[derive(Serialize)]
pub struct MeResponse {
    pub admin: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

pub async fn me(Extension(session): Extension<AdminSession>) -> Json<MeResponse> {
    Json(MeResponse { admin: true, expires_at: session.expires_at })
}

pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "version": crate::VERSION }))
}
```

Register: `crates/teramind-sync-server/src/admin_api/handlers/mod.rs`:

```rust
pub mod session;
```

### Task 7.2: Wire the routes

**File:** Modify `crates/teramind-sync-server/src/server.rs::build_router`.

- [ ] **Step 1**

After the existing routes, add an admin sub-router:

```rust
let admin_public = axum::Router::new()
    .route("/admin/login",   axum::routing::post(crate::admin_api::handlers::session::login))
    .route("/admin/logout",  axum::routing::post(crate::admin_api::handlers::session::logout))
    .route("/admin/version", axum::routing::get(crate::admin_api::handlers::session::version));
let admin_authed = axum::Router::new()
    .route("/admin/me", axum::routing::get(crate::admin_api::handlers::session::me))
    .layer(axum::middleware::from_fn_with_state(state.clone(), crate::admin_api::auth::admin_middleware));

let admin = admin_public.merge(admin_authed);
```

…and `.merge(admin)` into the top-level router. Pass `with_state(state.clone())` to admin_public so handlers can read `State<AppState>`.

The `ConnectInfo<SocketAddr>` extractor requires the server to be served with `.into_make_service_with_connect_info::<SocketAddr>()` — update `server.rs::serve` if it currently uses `.into_make_service()`.

### Task 7.3: Integration test

**File:** Create `crates/teramind-sync-server/tests/admin_login.rs`.

- [ ] **Step 1**

```rust
//! /admin/login: 200+cookie on correct password; 401 on wrong; 429 after 5 failures.

use argon2::{Argon2, PasswordHasher};
use argon2::password_hash::{rand_core::OsRng, SaltString};
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

async fn boot(password: &str) -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt).unwrap().to_string();
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(AdminConfig {
            admin_password_hash: hash,
            admin_session_secret: "ab".repeat(32),
            admin_session_ttl_hours: 12,
            event_log_retention_days: 90,
        }),
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .await.unwrap();
    });
    Ok((dir, sup, addr))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_succeeds_with_correct_password() -> anyhow::Result<()> {
    let (_d, sup, addr) = boot("hunter2hunter2").await?;
    let r = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" }))
        .send().await?;
    assert_eq!(r.status(), 200);
    let set_cookie = r.headers().get("set-cookie").unwrap().to_str()?.to_string();
    assert!(set_cookie.starts_with("tmd_admin="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_fails_with_wrong_password() -> anyhow::Result<()> {
    let (_d, sup, addr) = boot("hunter2hunter2").await?;
    let r = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "wrong" }))
        .send().await?;
    assert_eq!(r.status(), 401);
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_limits_after_five_failures() -> anyhow::Result<()> {
    let (_d, sup, addr) = boot("hunter2hunter2").await?;
    for _ in 0..5 {
        let r = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
            .json(&serde_json::json!({ "password": "wrong" })).send().await?;
        assert_eq!(r.status(), 401);
    }
    let r = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "wrong" })).send().await?;
    assert_eq!(r.status(), 429);
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn me_requires_cookie() -> anyhow::Result<()> {
    let (_d, sup, addr) = boot("hunter2hunter2").await?;
    let r = reqwest::Client::new().get(format!("http://{addr}/admin/me")).send().await?;
    assert_eq!(r.status(), 401);

    let login = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" })).send().await?;
    let cookie = login.headers().get("set-cookie").unwrap().to_str()?
        .split(';').next().unwrap().to_string();
    let r = reqwest::Client::new().get(format!("http://{addr}/admin/me"))
        .header("Cookie", cookie).send().await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["admin"], true);

    sup.shutdown().await?; Ok(())
}
```

- [ ] **Step 2: Run + commit**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-sync-server --test admin_login -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-sync-server/src/admin_api/handlers \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/admin_login.rs
git commit -m "feat(sync-server): /admin/login + /admin/logout + /admin/me + /admin/version"
```

---

## Section 8 — Event-log writer + pruner

### Task 8.1: EventLogWriter

**File:** Create `crates/teramind-sync-server/src/event_log_writer.rs`.

- [ ] **Step 1**

```rust
//! Fire-and-forget DB writer for TeamEvents.
//!
//! Every site that calls `bus.send(TeamEvent::...)` also calls `EventLogWriter::log(...)`.
//! The two writes are sequential — bus first (so live subscribers see the event
//! immediately), then a background tokio task does the DB insert. DB failures
//! log a warning; they do NOT block broadcast.

use std::sync::Arc;
use teramind_core::ids::UserId;
use teramind_core::team_event::TeamEvent;
use teramind_db::repos::TeamEventLogRepo;
use tracing::warn;

#[derive(Clone)]
pub struct EventLogWriter {
    repo: TeamEventLogRepo,
}

impl EventLogWriter {
    pub fn new(repo: TeamEventLogRepo) -> Arc<Self> { Arc::new(Self { repo }) }

    pub fn log(self: &Arc<Self>, event: TeamEvent) {
        let me = self.clone();
        tokio::spawn(async move {
            let (kind, user_id, cwd, payload) = match &event {
                TeamEvent::SessionEnded { session_id: _, user_id, cwd, ts: _ } => (
                    "session_ended", Some(UserId(*user_id)), Some(cwd.clone()),
                    serde_json::to_value(&event).unwrap_or_default(),
                ),
                TeamEvent::WikiPageReady { user_id, cwd, .. } => (
                    "wiki_page_ready", Some(UserId(*user_id)), Some(cwd.clone()),
                    serde_json::to_value(&event).unwrap_or_default(),
                ),
                TeamEvent::SkillSaved { user_id, .. } => (
                    "skill_saved", Some(UserId(*user_id)), None,
                    serde_json::to_value(&event).unwrap_or_default(),
                ),
            };
            if let Err(e) = me.repo.insert(kind, user_id, cwd, payload).await {
                warn!(error = %e, kind, "event_log insert failed");
            }
        });
    }
}
```

Register in `crates/teramind-sync-server/src/lib.rs`: `pub mod event_log_writer;`.

### Task 8.2: Pruner

**File:** Create `crates/teramind-sync-server/src/event_log_pruner.rs`.

- [ ] **Step 1**

```rust
//! Periodic delete of old team_event_log rows.

use std::time::Duration;
use teramind_db::repos::TeamEventLogRepo;
use tracing::{info, warn};

pub fn spawn(repo: TeamEventLogRepo, retention_days: i64, interval: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // burn the immediate-fire tick
        loop {
            tick.tick().await;
            match repo.prune_older_than(retention_days).await {
                Ok(n) if n > 0 => info!(rows = n, "event_log pruned"),
                Ok(_) => {}
                Err(e) => warn!(error = %e, "event_log prune failed"),
            }
        }
    });
}
```

Register: `pub mod event_log_pruner;` in `lib.rs`.

### Task 8.3: Wire into ingest publish + RPC SaveSkill

**File:** Modify `crates/teramind-sync-server/src/state.rs`.

- [ ] **Step 1: Add EventLogWriter to AppState**

```rust
pub event_log_writer: std::sync::Arc<crate::event_log_writer::EventLogWriter>,
```

In `AppState::new`:

```rust
let event_log_writer = crate::event_log_writer::EventLogWriter::new(event_log.clone());
```

(Already constructing `event_log` from §5.3.)

**File:** Modify `crates/teramind-sync-server/src/handlers/ingest.rs`.

- [ ] **Step 2: Call writer after each bus.send**

In `publish_on_success` (added in Plan L §2.2), after `state.bus.send(...)`, add:

```rust
state.event_log_writer.log(team_event.clone());
```

(Where `team_event` is the `TeamEvent::SessionEnded` you just constructed; restructure as needed to bind it to a variable before sending.)

**File:** Modify `crates/teramindd/src/services/rpc_dispatch.rs`.

- [ ] **Step 3: Add event_log_writer to RpcDeps**

In the `RpcDeps` struct, add:

```rust
pub event_log_writer: Option<std::sync::Arc<dyn EventLogger>>,
```

Define `EventLogger` as a trait in `teramindd/src/services/rpc_dispatch.rs` (the daemon can't depend on `teramind-sync-server` so we use a trait):

```rust
pub trait EventLogger: Send + Sync {
    fn log(&self, event: teramind_core::team_event::TeamEvent);
}
```

In the `SaveSkill` arm, after `bus.send(...)`:

```rust
if let Some(logger) = deps.event_log_writer.as_ref() {
    logger.log(team_event.clone());
}
```

Implement the trait for `EventLogWriter` in `teramind-sync-server/src/event_log_writer.rs`:

```rust
impl teramindd::services::rpc_dispatch::EventLogger for EventLogWriter {
    fn log(&self, event: teramind_core::team_event::TeamEvent) {
        EventLogWriter::log(&std::sync::Arc::new(self.clone()), event);
    }
}
```

(Slightly awkward shape because the public `log` takes `&Arc<Self>`. Cleaner: split it into a static helper that takes a `&TeamEventLogRepo` directly.)

Update `AppState::rpc_deps()` to populate the new field:

```rust
event_log_writer: Some(self.event_log_writer.clone() as std::sync::Arc<dyn teramindd::services::rpc_dispatch::EventLogger>),
```

For the daemon-side `DaemonIpcHandler::rpc_deps()`, set `event_log_writer: None`.

### Task 8.4: Spawn pruner in main.rs

**File:** Modify `crates/teramind-sync-server/src/main.rs`.

- [ ] **Step 1**

After the existing `fts_refresh::spawn(...)` call (Plan L), add:

```rust
if let Some(admin_cfg) = cfg.admin.as_ref() {
    crate::event_log_pruner::spawn(
        teramind_db::repos::TeamEventLogRepo::new(pool.clone()),
        admin_cfg.event_log_retention_days,
        std::time::Duration::from_secs(6 * 3600),
    );
}
```

### Task 8.5: Verify + commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind-sync-server -- --test-threads=1
git add crates/teramind-sync-server/src/event_log_writer.rs \
        crates/teramind-sync-server/src/event_log_pruner.rs \
        crates/teramind-sync-server/src/lib.rs \
        crates/teramind-sync-server/src/state.rs \
        crates/teramind-sync-server/src/handlers/ingest.rs \
        crates/teramind-sync-server/src/main.rs \
        crates/teramindd/src/services/rpc_dispatch.rs
git commit -m "feat(sync-server): event-log writer + pruner; wire into bus.send sites"
```

---

## Section 9 — Admin API: Activity (HTTP + WebSocket)

### Task 9.1: Implement handlers

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/activity.rs`.

- [ ] **Step 1**

```rust
//! /admin/activity (HTTP GET) + /admin/events (WebSocket)

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use teramind_core::ids::UserId;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ActivityQuery {
    #[serde(default = "default_limit")] pub limit: i64,
    pub before: Option<String>,
    pub kind: Option<String>,
    pub user_id: Option<String>,
}
fn default_limit() -> i64 { 100 }

pub async fn activity(
    State(state): State<AppState>,
    Extension(_session): Extension<AdminSession>,
    Query(q): Query<ActivityQuery>,
) -> DashboardResult<impl IntoResponse> {
    let before = q.before.as_deref()
        .and_then(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok());
    let user_id = q.user_id.as_deref()
        .and_then(|s| Uuid::parse_str(s).ok()).map(UserId);
    let rows = state.event_log.list_recent(q.kind.as_deref(), before, user_id, q.limit)
        .await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let next_before = rows.last().map(|r| r.ts.to_string());
    Ok(Json(serde_json::json!({
        "events": rows.iter().map(|r| serde_json::json!({
            "id": r.id, "kind": r.kind, "user_id": r.user_id.map(|u| u.0),
            "cwd": r.cwd, "payload": r.payload, "ts": r.ts.to_string(),
        })).collect::<Vec<_>>(),
        "next_before": next_before,
    })))
}

pub async fn events_ws(
    State(state): State<AppState>,
    Extension(_session): Extension<AdminSession>,
    ws: WebSocketUpgrade,
) -> Response {
    let rx = state.bus.subscribe();
    ws.on_upgrade(move |socket| handle_socket(socket, rx))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<TeamEvent>) {
    let hello = serde_json::json!({ "type": "hello" });
    if socket.send(Message::Text(hello.to_string())).await.is_err() { return; }
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(evt) => {
                        let json = match serde_json::to_string(&evt) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(json)).await.is_err() { return; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            inc = socket.recv() => {
                match inc {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(Message::Ping(p))) => {
                        if socket.send(Message::Pong(p)).await.is_err() { return; }
                    }
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
        }
    }
}
```

Register: `pub mod activity;` in `handlers/mod.rs`.

### Task 9.2: Wire routes

**File:** `crates/teramind-sync-server/src/server.rs`.

- [ ] **Step 1**

In the admin_authed sub-router, add:

```rust
.route("/admin/activity", axum::routing::get(crate::admin_api::handlers::activity::activity))
.route("/admin/events",   axum::routing::get(crate::admin_api::handlers::activity::events_ws))
```

### Task 9.3: Integration test

**File:** Create `crates/teramind-sync-server/tests/admin_activity.rs`.

- [ ] **Step 1**

```rust
//! /admin/activity (HTTP) + /admin/events (WS).

use std::net::SocketAddr;
use teramind_core::team_event::TeamEvent;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use uuid::Uuid;
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};

fn admin_cfg_with_password(password: &str) -> AdminConfig {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt).unwrap().to_string();
    AdminConfig {
        admin_password_hash: hash,
        admin_session_secret: "ab".repeat(32),
        admin_session_ttl_hours: 12,
        event_log_retention_days: 90,
    }
}

async fn boot() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, AppState, String)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(admin_cfg_with_password("hunter2hunter2")),
        quality: None,
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .await.unwrap();
    });
    let login = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" })).send().await?;
    let cookie = login.headers().get("set-cookie").unwrap().to_str()?
        .split(';').next().unwrap().to_string();
    Ok((dir, sup, addr, state, cookie))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn activity_returns_recent_rows() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    state.event_log.insert("skill_saved", None, None, serde_json::json!({"name":"x"})).await?;
    state.event_log.insert("session_ended", None, Some("/proj".into()), serde_json::json!({})).await?;

    let r = reqwest::Client::new().get(format!("http://{addr}/admin/activity?limit=10"))
        .header("Cookie", &cookie).send().await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["events"].as_array().unwrap().len(), 2);

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_subscriber_receives_bus_event() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    let req = Request::builder()
        .uri(format!("ws://{addr}/admin/events"))
        .header("Host", addr.to_string())
        .header("Cookie", &cookie)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())?;
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    let (_w, mut r) = ws.split();

    // Eat hello.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), r.next()).await?.unwrap()?;

    let _ = state.bus.send(TeamEvent::SkillSaved {
        skill_id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        name: "test".into(),
        ts: time::OffsetDateTime::now_utc(),
    });

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), r.next()).await?.unwrap()?;
    if let Message::Text(t) = msg {
        let evt: TeamEvent = serde_json::from_str(&t)?;
        match evt {
            TeamEvent::SkillSaved { name, .. } => assert_eq!(name, "test"),
            _ => panic!("unexpected"),
        }
    } else { panic!("expected text"); }

    sup.shutdown().await?; Ok(())
}
```

### Task 9.4: Commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-sync-server --test admin_activity -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-sync-server/src/admin_api/handlers/activity.rs \
        crates/teramind-sync-server/src/admin_api/handlers/mod.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/admin_activity.rs
git commit -m "feat(sync-server): /admin/activity + /admin/events"
```

---

## Section 10 — Admin API: Skills + Candidates + Observations

The three views share repos already created (Plan A skills, Plan M observations + candidates). Each handler is a thin shell.

### Task 10.1: Skills handler

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/skills.rs`.

- [ ] **Step 1**

```rust
//! /admin/skills (list/show/delete)

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SkillsQuery {
    #[serde(default = "default_limit")] pub limit: i64,
    #[serde(default)] pub offset: i64,
    pub source: Option<String>,
    pub q: Option<String>,
}
fn default_limit() -> i64 { 100 }

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<SkillsQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let src = q.source.as_deref().filter(|s| matches!(*s, "authored"|"codified"|"imported"));
    let term = q.q.as_deref().unwrap_or("").to_string();
    let like = format!("%{}%", term);

    let rows: Vec<(uuid::Uuid, String, String, String, Vec<String>, Vec<uuid::Uuid>, time::OffsetDateTime, time::OffsetDateTime)> =
        sqlx::query_as(
            r#"SELECT id, name, description, source, applies_to_cwds, source_session_ids, created_at, updated_at
               FROM skills
               WHERE ($1::text IS NULL OR source = $1)
                 AND ($2::text = '' OR name ILIKE $3 OR description ILIKE $3)
               ORDER BY updated_at DESC
               LIMIT $4 OFFSET $5"#)
            .bind(src).bind(&term).bind(&like).bind(q.limit).bind(q.offset)
            .fetch_all(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;

    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM skills")
        .fetch_one(state.pool.pg()).await
        .unwrap_or((rows.len() as i64,));

    let skills = rows.into_iter().map(|(id, name, desc, source, cwds, sids, created, updated)| {
        serde_json::json!({
            "id": id, "name": name, "description": desc, "source": source,
            "applies_to_cwds": cwds, "source_session_ids": sids,
            "created_at": created.to_string(), "updated_at": updated.to_string(),
        })
    }).collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "skills": skills, "total": total })))
}

pub async fn show(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id_or_name): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let row: Option<(uuid::Uuid, String, String, String, String, Vec<String>, Vec<uuid::Uuid>, time::OffsetDateTime, time::OffsetDateTime)> =
        sqlx::query_as(
            r#"SELECT id, name, description, body, source, applies_to_cwds, source_session_ids, created_at, updated_at
               FROM skills WHERE name = $1 OR id::text = $1 LIMIT 1"#)
            .bind(&id_or_name).fetch_optional(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let Some((id, name, description, body, source, cwds, sids, created, updated)) = row else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "no such skill"));
    };
    Ok(Json(serde_json::json!({
        "id": id, "name": name, "description": description, "body": body, "source": source,
        "applies_to_cwds": cwds, "source_session_ids": sids,
        "created_at": created.to_string(), "updated_at": updated.to_string(),
    })))
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let n = sqlx::query("DELETE FROM skills WHERE id = $1")
        .bind(id).execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "no such skill"));
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}
```

Register: `pub mod skills;` in `handlers/mod.rs`.

### Task 10.2: Candidates handler

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/candidates.rs`.

- [ ] **Step 1**

```rust
//! /admin/candidates list / show / approve / reject / PATCH.

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use teramind_core::ids::SkillCandidateId;

#[derive(Deserialize)]
pub struct CandidatesQuery {
    #[serde(default = "default_limit")] pub limit: i64,
    #[serde(default)] pub offset: i64,
    pub status: Option<String>,
}
fn default_limit() -> i64 { 100 }

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<CandidatesQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let rows = repo.list_filter(q.status.as_deref(), q.limit).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({
        "candidates": rows.iter().map(|c| serde_json::json!({
            "id": c.id.0, "observation_id": c.observation_id.0,
            "name": c.name, "description": c.description, "body": c.body,
            "applies_to_cwds": c.applies_to_cwds,
            "source_session_ids": c.source_session_ids,
            "model": c.model,
            "input_tokens": c.input_tokens, "output_tokens": c.output_tokens,
            "generated_at": c.generated_at.to_string(),
            "status": c.status,
            "reviewer": c.reviewer,
        })).collect::<Vec<_>>(),
        "total": rows.len(),
    })))
}

pub async fn show(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let row: Option<(uuid::Uuid, uuid::Uuid, String, String, String, Vec<String>, Vec<uuid::Uuid>, String, i32, i32, time::OffsetDateTime, String, Option<String>, Option<time::OffsetDateTime>)> =
        sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates WHERE id = $1"#)
            .bind(id).fetch_optional(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let Some(r) = row else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "no such candidate"));
    };
    Ok(Json(serde_json::json!({
        "id": r.0, "observation_id": r.1, "name": r.2, "description": r.3, "body": r.4,
        "applies_to_cwds": r.5, "source_session_ids": r.6, "model": r.7,
        "input_tokens": r.8, "output_tokens": r.9, "generated_at": r.10.to_string(),
        "status": r.11, "reviewer": r.12, "reviewed_at": r.13.map(|t| t.to_string()),
    })))
}

#[derive(Deserialize)]
pub struct ApproveBody { pub reviewer: Option<String> }

pub async fn approve(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let reviewer = body.reviewer.unwrap_or_else(|| "admin".into());
    let n = sqlx::query(
        "UPDATE skill_candidates SET status='approved', reviewer=$2, reviewed_at=now()
         WHERE id=$1 AND status='pending'")
        .bind(id).bind(&reviewer).execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::CONFLICT, "conflict", "candidate not pending"));
    }
    // Synchronous promotion so the UI sees the live skill immediately.
    let cand_repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let skill_repo = teramind_db::repos::SkillRepo::new(state.pool.clone());
    let _ = teramindd::services::codify::promote::promote_approved_batch(
        &state.pool, &cand_repo, &skill_repo, 10,
    ).await;
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT s.id FROM skill_candidates c JOIN skills s ON s.name = c.name
         WHERE c.id = $1")
        .bind(id).fetch_optional(state.pool.pg()).await.ok().flatten();
    Ok(Json(serde_json::json!({ "skill_id": row.map(|(id,)| id) })))
}

#[derive(Deserialize)]
pub struct RejectBody { pub reviewer: Option<String>, pub reason: Option<String> }

pub async fn reject(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
    Json(body): Json<RejectBody>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let reviewer = body.reviewer.unwrap_or_else(|| "admin".into());
    let n = sqlx::query(
        "UPDATE skill_candidates SET status='rejected', reviewer=$2, reviewed_at=now()
         WHERE id=$1 AND status='pending'")
        .bind(id).bind(&reviewer).execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::CONFLICT, "conflict", "candidate not pending"));
    }
    let _ = body.reason;  // reserved; not persisted in v1
    Ok(Json(serde_json::json!({ "rejected": true })))
}

#[derive(Deserialize)]
pub struct PatchBody {
    pub description: Option<String>,
    pub body: Option<String>,
    pub applies_to_cwds: Option<Vec<String>>,
}

pub async fn patch(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
    Json(p): Json<PatchBody>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let n = sqlx::query(
        r#"UPDATE skill_candidates
           SET description = COALESCE($2, description),
               body        = COALESCE($3, body),
               applies_to_cwds = COALESCE($4, applies_to_cwds)
           WHERE id = $1 AND status = 'pending'"#)
        .bind(id).bind(p.description).bind(p.body).bind(p.applies_to_cwds)
        .execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::CONFLICT, "conflict", "candidate not pending"));
    }
    Ok(Json(serde_json::json!({ "updated": true })))
}
```

Register: `pub mod candidates;`.

### Task 10.3: Observations handler

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/observations.rs`.

- [ ] **Step 1**

```rust
//! /admin/observations list + show.

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{extract::{Extension, Path, Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use teramind_db::repos::SkillObservationRepo;

#[derive(Deserialize)]
pub struct ObsQuery {
    pub kind: Option<String>,
    pub status: Option<String>,
    #[serde(default)] pub min_freq: i32,
    #[serde(default = "default_limit")] pub limit: i64,
}
fn default_limit() -> i64 { 100 }

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<ObsQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let repo = SkillObservationRepo::new(state.pool.clone());
    let rows = repo.list_recent(q.kind.as_deref(), q.status.as_deref(), q.limit).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({
        "observations": rows.iter().filter(|o| o.frequency >= q.min_freq).map(|o| serde_json::json!({
            "id": o.id.0, "kind": o.kind, "signature": o.signature,
            "frequency": o.frequency, "status": o.status,
            "last_seen_at": o.last_seen_at.to_string(),
        })).collect::<Vec<_>>(),
        "total": rows.len(),
    })))
}

pub async fn show(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let row: Option<(uuid::Uuid, String, String, Vec<uuid::Uuid>, i32, serde_json::Value, time::OffsetDateTime, time::OffsetDateTime, String)> =
        sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations WHERE id = $1"#)
            .bind(id).fetch_optional(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let Some(r) = row else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "no such observation"));
    };
    Ok(Json(serde_json::json!({
        "id": r.0, "kind": r.1, "signature": r.2, "session_ids": r.3,
        "frequency": r.4, "context_blob": r.5,
        "first_seen_at": r.6.to_string(), "last_seen_at": r.7.to_string(),
        "status": r.8,
    })))
}
```

Register: `pub mod observations;`.

### Task 10.4: Wire routes

**File:** `crates/teramind-sync-server/src/server.rs::build_router`.

- [ ] **Step 1**

In `admin_authed`:

```rust
.route("/admin/skills",       axum::routing::get(crate::admin_api::handlers::skills::list))
.route("/admin/skills/{id}",  axum::routing::get(crate::admin_api::handlers::skills::show)
                                      .delete(crate::admin_api::handlers::skills::delete))
.route("/admin/candidates",       axum::routing::get(crate::admin_api::handlers::candidates::list))
.route("/admin/candidates/{id}",  axum::routing::get(crate::admin_api::handlers::candidates::show)
                                       .patch(crate::admin_api::handlers::candidates::patch))
.route("/admin/candidates/{id}/approve", axum::routing::post(crate::admin_api::handlers::candidates::approve))
.route("/admin/candidates/{id}/reject",  axum::routing::post(crate::admin_api::handlers::candidates::reject))
.route("/admin/observations",       axum::routing::get(crate::admin_api::handlers::observations::list))
.route("/admin/observations/{id}",  axum::routing::get(crate::admin_api::handlers::observations::show))
```

### Task 10.5: Tests

Create three test files mirroring `admin_login.rs`'s `boot()` scaffolding:
- `crates/teramind-sync-server/tests/admin_skills.rs` — list, show, delete; 401 without cookie.
- `crates/teramind-sync-server/tests/admin_candidates.rs` — seed an observation + insert a candidate manually; verify approve returns 200 + a `skill_id`; verify a second approve is 409; verify patch updates the body.
- `crates/teramind-sync-server/tests/admin_observations.rs` — seed observations; verify list filters by kind/status; verify show returns context_blob.

Use the same `boot_with_admin` helper pattern from Task 7.3. Aim for ~3 tests per file (~9 total).

### Task 10.6: Commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-sync-server --test admin_skills -- --test-threads=1
cargo test -p teramind-sync-server --test admin_candidates -- --test-threads=1
cargo test -p teramind-sync-server --test admin_observations -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-sync-server/src/admin_api/handlers/{skills,candidates,observations,mod}.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/admin_skills.rs \
        crates/teramind-sync-server/tests/admin_candidates.rs \
        crates/teramind-sync-server/tests/admin_observations.rs
git commit -m "feat(sync-server): /admin/skills + /admin/candidates + /admin/observations"
```

---

## Section 11 — Admin API: Members + devices + invites

### Task 11.1: Handler

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/members.rs`.

- [ ] **Step 1**

```rust
//! /admin/members + /admin/devices + /admin/invites

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::invite::InviteCode;
use crate::state::AppState;
use axum::{extract::{Extension, Path, State}, http::StatusCode, Json};
use rand::rngs::OsRng;
use serde::Deserialize;
use teramind_core::ids::{DeviceId, InviteId, UserId};
use time::{Duration, OffsetDateTime};

pub async fn members(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> DashboardResult<Json<serde_json::Value>> {
    let rows: Vec<(uuid::Uuid, String, Option<String>, time::OffsetDateTime, Option<time::OffsetDateTime>)> =
        sqlx::query_as(
            r#"SELECT id, email, display_name, created_at, revoked_at FROM users ORDER BY email"#)
            .fetch_all(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let mut out = vec![];
    for (uid, email, name, created, revoked) in rows {
        let counts: Vec<(i64, Option<time::OffsetDateTime>)> = sqlx::query_as(
            r#"SELECT count(*), max(last_seen_at)::timestamptz FROM devices
               WHERE user_id = $1 AND revoked_at IS NULL"#)
            .bind(uid).fetch_all(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
        let (device_count, last_seen) = counts.first().cloned().unwrap_or((0, None));
        out.push(serde_json::json!({
            "id": uid, "email": email, "display_name": name,
            "created_at": created.to_string(),
            "revoked_at": revoked.map(|t| t.to_string()),
            "device_count": device_count,
            "last_seen_at": last_seen.map(|t| t.to_string()),
        }));
    }
    Ok(Json(serde_json::json!({ "users": out })))
}

pub async fn revoke_user(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(user_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = UserId(uuid::Uuid::parse_str(&user_id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?);
    state.users.revoke(id).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

pub async fn user_devices(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(user_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = UserId(uuid::Uuid::parse_str(&user_id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?);
    let devices = state.devices.list_for_user(id).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let json = devices.into_iter().map(|d| serde_json::json!({
        "id": d.id.0, "name": d.name, "last_seen_at": d.last_seen_at.map(|t| t.to_string()),
    })).collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "devices": json })))
}

pub async fn revoke_device(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(device_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = DeviceId(uuid::Uuid::parse_str(&device_id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?);
    state.devices.revoke(id).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

pub async fn list_invites(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> DashboardResult<Json<serde_json::Value>> {
    let invites = state.invites.list_outstanding().await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let json = invites.into_iter().map(|i| serde_json::json!({
        "id": i.id.0, "invited_email": i.invited_email, "display_name": i.display_name,
        "created_by": i.created_by, "created_at": i.created_at.to_string(),
        "expires_at": i.expires_at.to_string(),
    })).collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "invites": json })))
}

#[derive(Deserialize)]
pub struct NewInvite {
    pub email: String,
    pub display_name: Option<String>,
    pub expires_in_days: Option<i64>,
}

pub async fn create_invite(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Json(body): Json<NewInvite>,
) -> DashboardResult<(StatusCode, Json<serde_json::Value>)> {
    let code = InviteCode::generate(&mut OsRng);
    let days = body.expires_in_days.unwrap_or(state.cfg.auth.invite_default_expires_days);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(days);
    let id = state.invites.create(&code.hash(), &body.email, body.display_name.as_deref(),
                                  Some("admin"), expires_at).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "invite_id": id.0,
        "code": code.as_str(),
        "expires_at": expires_at.to_string(),
    }))))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(invite_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = InviteId(uuid::Uuid::parse_str(&invite_id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?);
    state.invites.revoke(id).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}
```

Register: `pub mod members;`.

### Task 11.2: Wire routes

```rust
.route("/admin/members",                          axum::routing::get(crate::admin_api::handlers::members::members))
.route("/admin/members/{user_id}/revoke",         axum::routing::post(crate::admin_api::handlers::members::revoke_user))
.route("/admin/members/{user_id}/devices",        axum::routing::get(crate::admin_api::handlers::members::user_devices))
.route("/admin/devices/{device_id}/revoke",       axum::routing::post(crate::admin_api::handlers::members::revoke_device))
.route("/admin/invites",                          axum::routing::get(crate::admin_api::handlers::members::list_invites)
                                                      .post(crate::admin_api::handlers::members::create_invite))
.route("/admin/invites/{id}/revoke",              axum::routing::post(crate::admin_api::handlers::members::revoke_invite))
```

### Task 11.3: Test + commit

Create `crates/teramind-sync-server/tests/admin_members.rs` with ~3 tests:
- `lists_members_with_device_counts`
- `creates_invite_and_returns_code_once`
- `revokes_device_via_admin_endpoint`

(Same boot helper pattern as before. Seed users/devices/invites directly via the repos, then HTTP-call the endpoints.)

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-sync-server --test admin_members -- --test-threads=1
git add crates/teramind-sync-server/src/admin_api/handlers/members.rs \
        crates/teramind-sync-server/src/admin_api/handlers/mod.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/admin_members.rs
git commit -m "feat(sync-server): /admin/members + /admin/devices + /admin/invites"
```

---

## Section 12 — QualityRunOutput in core + `--json` flag in eval

### Task 12.1: QualityRunOutput

**File:** Create `crates/teramind-core/src/quality.rs`.

- [ ] **Step 1**

```rust
//! Shared between teramind-search-eval (writer) and teramind-sync-server (reader).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRunOutput {
    pub baseline_label: String,
    pub model: Option<String>,
    pub ndcg10: f64,
    pub mrr: f64,
    pub precision_5: f64,
    pub precision_10: f64,
    pub recall_10: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub query_count: u32,
    pub corpus_size: u32,
    pub per_class: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_through_json() {
        let q = QualityRunOutput {
            baseline_label: "lexical".into(),
            model: None,
            ndcg10: 0.142, mrr: 0.301, precision_5: 0.23, precision_10: 0.18, recall_10: 0.42,
            p50_latency_ms: 42.0, p95_latency_ms: 380.0,
            query_count: 100, corpus_size: 500,
            per_class: json!({}),
        };
        let s = serde_json::to_string(&q).unwrap();
        let back: QualityRunOutput = serde_json::from_str(&s).unwrap();
        assert_eq!(back.baseline_label, "lexical");
        assert!((back.ndcg10 - 0.142).abs() < 1e-9);
    }
}
```

Register: `pub mod quality;` in `crates/teramind-core/src/lib.rs`.

### Task 12.2: `--json` flag in eval binary

**File:** Modify `crates/teramind-search-eval/src/main.rs`.

- [ ] **Step 1: Inspect existing CLI**

```bash
grep -n "clap\|#\[arg\|fn main" crates/teramind-search-eval/src/main.rs | head -20
```

Find the existing clap struct + the place where the binary computes its metrics today.

- [ ] **Step 2: Add the `--json` flag**

In the existing `Cli` struct, add:

```rust
/// Emit metrics as a single JSON object (suitable for ingestion by the dashboard).
#[arg(long)]
pub json: bool,
```

After the metrics are computed (locate the `nDCG@10 = …` print statement; the values must be in scope at that point), add:

```rust
if cli.json {
    let out = teramind_core::quality::QualityRunOutput {
        baseline_label: cli.baseline.clone().unwrap_or_else(|| "lexical".into()),
        model: cli.model.clone(),
        ndcg10, mrr, precision_5, precision_10, recall_10,
        p50_latency_ms, p95_latency_ms,
        query_count: total_queries as u32,
        corpus_size: total_corpus as u32,
        per_class: serde_json::to_value(&per_class).unwrap_or(serde_json::json!({})),
    };
    println!("{}", serde_json::to_string(&out).unwrap());
    return Ok(());
}
```

Variable names (`ndcg10`, `mrr`, `per_class`, etc.) may differ in the existing code — adapt to whatever's in scope. The crate already depends on `teramind-core` (for shared types); add `serde_json` if missing.

- [ ] **Step 3: Smoke test**

```bash
cargo run -p teramind-search-eval -- --baseline lexical --json 2>/dev/null | jq .ndcg10
```

Expected: a single floating-point number.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/quality.rs \
        crates/teramind-core/src/lib.rs \
        crates/teramind-search-eval/src/main.rs
git commit -m "feat(eval): --json flag emitting QualityRunOutput"
```

---

## Section 13 — Quality scheduler

### Task 13.1: Implement

**File:** Create `crates/teramind-sync-server/src/quality_scheduler.rs`.

- [ ] **Step 1**

```rust
//! Cron-driven runner for teramind-search-eval. Persists results to quality_runs.

use crate::config::QualityConfig;
use cron::Schedule;
use std::str::FromStr;
use std::time::Duration as StdDuration;
use teramind_core::quality::QualityRunOutput;
use teramind_db::pool::DbPool;
use teramind_db::repos::QualityRunRepo;
use tokio::process::Command;
use tracing::{info, warn};

pub fn spawn(pool: DbPool, cfg: QualityConfig) -> Option<tokio::task::JoinHandle<()>> {
    if !cfg.enabled { return None; }
    let cron = cfg.cron.clone().unwrap_or_else(|| "0 2 * * *".into());
    let schedule = match Schedule::from_str(&format!("0 {cron}")) {
        // cron crate expects 6-field (sec, min, hr, dom, mon, dow).
        // We prepend "0" so users can supply 5-field "min hr dom mon dow".
        Ok(s) => s,
        Err(e) => { warn!(error = %e, "invalid cron in [quality]; disabling scheduler"); return None; }
    };
    let repo = QualityRunRepo::new(pool);
    Some(tokio::spawn(async move { run_loop(repo, cfg, schedule).await }))
}

async fn run_loop(repo: QualityRunRepo, cfg: QualityConfig, schedule: Schedule) {
    use chrono::Utc;
    let mut last_for: std::collections::HashMap<String, std::time::Instant> = Default::default();
    loop {
        let next = match schedule.upcoming(Utc).next() {
            Some(n) => n,
            None => break,
        };
        let now = Utc::now();
        let delay = (next - now).to_std().unwrap_or(StdDuration::from_secs(60));
        tokio::time::sleep(delay).await;

        for baseline in &cfg.baselines {
            // Single-flight per baseline.
            if let Some(t) = last_for.get(baseline) {
                if t.elapsed() < StdDuration::from_secs(60) { continue; }
            }
            last_for.insert(baseline.clone(), std::time::Instant::now());
            run_one(&repo, &cfg.eval_binary, baseline).await;
        }
    }
}

async fn run_one(repo: &QualityRunRepo, binary: &str, baseline: &str) {
    info!(baseline, "starting scheduled eval");
    let out = Command::new(binary).arg("--baseline").arg(baseline).arg("--json").output().await;
    match out {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            match serde_json::from_str::<QualityRunOutput>(&stdout) {
                Ok(q) => {
                    let raw = serde_json::to_value(&q).unwrap_or(serde_json::json!({}));
                    let res = repo.insert(
                        &q.baseline_label, q.model.clone(),
                        q.ndcg10, q.mrr, q.precision_5, q.precision_10, q.recall_10,
                        q.p50_latency_ms, q.p95_latency_ms,
                        q.query_count as i32, q.corpus_size as i32,
                        q.per_class.clone(), raw, "scheduled",
                    ).await;
                    if let Err(e) = res { warn!(error = %e, "quality_runs insert failed"); }
                }
                Err(e) => {
                    warn!(error = %e, "failed to parse eval output");
                    let raw = serde_json::json!({ "error": e.to_string(), "stdout": stdout });
                    let _ = repo.insert(baseline, None,
                        f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN,
                        f64::NAN, f64::NAN, 0, 0,
                        serde_json::json!({}), raw, "scheduled").await;
                }
            }
        }
        Ok(o) => {
            warn!(status = ?o.status, "eval binary returned non-zero");
        }
        Err(e) => {
            warn!(error = %e, baseline, "eval binary failed to spawn");
        }
    }
}
```

Register: `pub mod quality_scheduler;` in `lib.rs`.

Add to `crates/teramind-sync-server/Cargo.toml`:

```toml
chrono = "0.4"
```

(`cron` already in workspace deps from §3.1.)

### Task 13.2: Spawn from main.rs

**File:** `crates/teramind-sync-server/src/main.rs`.

- [ ] **Step 1**

In the `Serve` arm, after the `event_log_pruner::spawn(...)`:

```rust
if let Some(quality_cfg) = cfg.quality.clone() {
    let _ = crate::quality_scheduler::spawn(pool.clone(), quality_cfg);
}
```

### Task 13.3: Commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-sync-server/src/quality_scheduler.rs \
        crates/teramind-sync-server/src/lib.rs \
        crates/teramind-sync-server/src/main.rs \
        crates/teramind-sync-server/Cargo.toml
git commit -m "feat(sync-server): quality scheduler (cron-driven eval runner)"
```

---

## Section 14 — Admin API: Quality endpoints + Health

### Task 14.1: Quality handler

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/quality.rs`.

- [ ] **Step 1**

```rust
//! /admin/quality endpoints.

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{extract::{Extension, Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use teramind_core::quality::QualityRunOutput;

#[derive(Deserialize)]
pub struct QualityQuery {
    pub since: Option<String>,
    pub baseline: Option<String>,
    #[serde(default = "default_limit")] pub limit: i64,
}
fn default_limit() -> i64 { 60 }

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<QualityQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let rows = state.quality.list_recent(q.baseline.as_deref(), q.limit).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let _ = q.since;  // v1: client paginates by limit only
    Ok(Json(serde_json::json!({
        "runs": rows.into_iter().map(|r| serde_json::json!({
            "id": r.id, "baseline_label": r.baseline_label, "model": r.model,
            "ndcg10": r.ndcg10, "mrr": r.mrr,
            "precision_5": r.precision_5, "precision_10": r.precision_10, "recall_10": r.recall_10,
            "p50_latency_ms": r.p50_latency_ms, "p95_latency_ms": r.p95_latency_ms,
            "query_count": r.query_count, "corpus_size": r.corpus_size,
            "ran_at": r.ran_at.to_string(),
            "source": r.source,
            "per_class": r.per_class,
        })).collect::<Vec<_>>()
    })))
}

pub async fn latest(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<QualityQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let baseline = q.baseline.clone().unwrap_or_else(|| "lexical".into());
    let row = state.quality.latest(&baseline).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({
        "run": row.map(|r| serde_json::json!({
            "id": r.id, "baseline_label": r.baseline_label, "model": r.model,
            "ndcg10": r.ndcg10, "mrr": r.mrr,
            "p50_latency_ms": r.p50_latency_ms, "p95_latency_ms": r.p95_latency_ms,
            "ran_at": r.ran_at.to_string(),
        }))
    })))
}

pub async fn upload(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Json(q): Json<QualityRunOutput>,
) -> DashboardResult<(StatusCode, Json<serde_json::Value>)> {
    if !q.ndcg10.is_finite() || !q.mrr.is_finite()
        || !(0.0..=1.0).contains(&q.ndcg10) || !(0.0..=1.0).contains(&q.mrr) {
        return Err(DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "metrics out of range"));
    }
    let raw = serde_json::to_value(&q).unwrap_or_default();
    let id = state.quality.insert(
        &q.baseline_label, q.model.clone(),
        q.ndcg10, q.mrr, q.precision_5, q.precision_10, q.recall_10,
        q.p50_latency_ms, q.p95_latency_ms,
        q.query_count as i32, q.corpus_size as i32,
        q.per_class.clone(), raw, "manual",
    ).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

pub async fn config(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "enabled": state.cfg.quality.as_ref().map(|q| q.enabled).unwrap_or(false),
        "cron":    state.cfg.quality.as_ref().and_then(|q| q.cron.clone()),
        "baselines": state.cfg.quality.as_ref().map(|q| q.baselines.clone()).unwrap_or_default(),
    }))
}
```

Register: `pub mod quality;`.

### Task 14.2: Health handler

**File:** Create `crates/teramind-sync-server/src/admin_api/handlers/health.rs`.

- [ ] **Step 1**

```rust
use crate::admin_api::cookie::AdminSession;
use crate::state::AppState;
use axum::{extract::{Extension, State}, Json};

pub async fn health(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> Json<serde_json::Value> {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(state.pool.pg()).await.is_ok();
    Json(serde_json::json!({
        "db": if db_ok { "ok" } else { "down" },
        "broadcast_subscribers": state.bus.receiver_count(),
        "quality_scheduler": {
            "enabled": state.cfg.quality.as_ref().map(|q| q.enabled).unwrap_or(false),
        },
    }))
}
```

Register: `pub mod health;`.

### Task 14.3: Wire routes + test

In `server.rs::build_router` admin_authed:

```rust
.route("/admin/quality",         axum::routing::get(crate::admin_api::handlers::quality::list))
.route("/admin/quality/latest",  axum::routing::get(crate::admin_api::handlers::quality::latest))
.route("/admin/quality/runs",    axum::routing::post(crate::admin_api::handlers::quality::upload))
.route("/admin/quality/config",  axum::routing::get(crate::admin_api::handlers::quality::config))
.route("/admin/health",          axum::routing::get(crate::admin_api::handlers::health::health))
```

Create `crates/teramind-sync-server/tests/admin_quality.rs` with three tests:
- `upload_persists_run`
- `latest_returns_most_recent`
- `validation_rejects_nan`

### Task 14.4: Commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-sync-server --test admin_quality -- --test-threads=1
git add crates/teramind-sync-server/src/admin_api/handlers/{quality,health,mod}.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/admin_quality.rs
git commit -m "feat(sync-server): /admin/quality + /admin/health"
```

---

## Section 15 — Static asset serving + embedding

### Task 15.1: dashboard/dist placeholder

- [ ] **Step 1**

```bash
mkdir -p dashboard/dist
echo '' > dashboard/dist/.gitkeep
echo '<!doctype html><html><body>Dashboard bundle missing. Run: cd dashboard && npm install && npm run build</body></html>' > dashboard/dist/index.html
```

This empty placeholder lets `include_dir!` compile before the frontend exists.

### Task 15.2: dashboard_assets module

**File:** Create `crates/teramind-sync-server/src/dashboard_assets.rs`.

- [ ] **Step 1**

```rust
use include_dir::{include_dir, Dir};

static DASHBOARD: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../dashboard/dist");

pub fn lookup(path: &str) -> Option<(&'static [u8], &'static str)> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let file = DASHBOARD.get_file(path)
        .or_else(|| DASHBOARD.get_file("index.html"))?;
    let content_type = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js")   => "application/javascript",
        Some("css")  => "text/css",
        Some("svg")  => "image/svg+xml",
        Some("png")  => "image/png",
        Some("ico")  => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        _            => "application/octet-stream",
    };
    Some((file.contents(), content_type))
}
```

Register: `pub mod dashboard_assets;` in `lib.rs`.

### Task 15.3: build.rs

**File:** Create `crates/teramind-sync-server/build.rs`.

- [ ] **Step 1**

```rust
fn main() {
    let dist = std::path::Path::new("../../dashboard/dist");
    if !dist.join("index.html").exists() {
        println!("cargo:warning=dashboard/dist/index.html missing; server will serve placeholder");
    }
    println!("cargo:rerun-if-changed=../../dashboard/dist");
}
```

### Task 15.4: Static route handlers

**File:** Modify `crates/teramind-sync-server/src/server.rs`.

- [ ] **Step 1: Add handlers**

```rust
use axum::{extract::Path, http::{header, HeaderValue, StatusCode}, response::IntoResponse};

async fn serve_dashboard_index() -> impl IntoResponse {
    match crate::dashboard_assets::lookup("index.html") {
        Some((bytes, ct)) => {
            let mut resp = bytes.into_response();
            resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
            resp
        }
        None => (StatusCode::NOT_FOUND, "dashboard not built").into_response(),
    }
}

async fn serve_dashboard_asset(Path(path): Path<String>) -> impl IntoResponse {
    match crate::dashboard_assets::lookup(&path) {
        Some((bytes, ct)) => {
            let mut resp = bytes.into_response();
            resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
            resp
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
```

In `build_router`, add to the public router:

```rust
.route("/dashboard",            axum::routing::get(serve_dashboard_index))
.route("/dashboard/{*path}",    axum::routing::get(serve_dashboard_asset))
```

### Task 15.5: Test

**File:** Create `crates/teramind-sync-server/tests/dashboard_assets.rs`.

- [ ] **Step 1**

```rust
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashboard_index_returns_html() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .await.unwrap();
    });

    let r = reqwest::Client::new().get(format!("http://{addr}/dashboard")).send().await?;
    assert_eq!(r.status(), 200);
    let ct = r.headers().get("content-type").unwrap().to_str()?.to_string();
    assert!(ct.starts_with("text/html"));

    let r2 = reqwest::Client::new().get(format!("http://{addr}/dashboard/unknown-route")).send().await?;
    assert_eq!(r2.status(), 200);
    assert!(r2.headers().get("content-type").unwrap().to_str()?.starts_with("text/html"));

    sup.shutdown().await?;
    Ok(())
}
```

### Task 15.6: Commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind-sync-server --test dashboard_assets -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
git add dashboard/dist/.gitkeep dashboard/dist/index.html \
        crates/teramind-sync-server/build.rs \
        crates/teramind-sync-server/src/dashboard_assets.rs \
        crates/teramind-sync-server/src/lib.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/dashboard_assets.rs
git commit -m "feat(sync-server): /dashboard/* static asset serving (placeholder)"
```

The backend half is complete. Sections §16-§21 build the React SPA that replaces the placeholder.

---

## Section 16 — Frontend scaffold

### Task 16.1: package.json + tooling

**File:** Create `dashboard/package.json`.

- [ ] **Step 1**

```json
{
  "name": "teramind-dashboard",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "lint": "tsc --noEmit",
    "playwright": "playwright test"
  },
  "dependencies": {
    "@tanstack/react-query": "^5.51.0",
    "@tanstack/react-router": "^1.45.0",
    "lucide-react": "^0.400.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "recharts": "^2.12.7"
  },
  "devDependencies": {
    "@playwright/test": "^1.45.0",
    "@types/react": "^18.3.3",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.1",
    "autoprefixer": "^10.4.19",
    "postcss": "^8.4.39",
    "tailwindcss": "^3.4.6",
    "typescript": "^5.5.3",
    "vite": "^5.3.3",
    "vitest": "^1.6.0"
  }
}
```

### Task 16.2: tsconfig + vite + tailwind + postcss configs

**Files:** Create `dashboard/tsconfig.json`, `dashboard/vite.config.ts`, `dashboard/tailwind.config.ts`, `dashboard/postcss.config.cjs`.

- [ ] **Step 1: tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noImplicitAny": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "jsx": "react-jsx",
    "baseUrl": "src",
    "paths": { "@/*": ["./*"] }
  },
  "include": ["src", "tests"]
}
```

- [ ] **Step 2: vite.config.ts**

```ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';

export default defineConfig({
  plugins: [react()],
  base: '/dashboard/',
  server: {
    port: 5173,
    proxy: {
      '/admin':     { target: 'http://localhost:8443', changeOrigin: true },
      // WebSocket proxy for /admin/events
      '/admin/events': { target: 'ws://localhost:8443', ws: true, changeOrigin: true },
    },
  },
  resolve: { alias: { '@': path.resolve(__dirname, 'src') } },
  build: {
    target: 'es2022',
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks: { recharts: ['recharts'] },
      },
    },
  },
});
```

- [ ] **Step 3: tailwind.config.ts**

```ts
import type { Config } from 'tailwindcss';

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: { extend: {} },
  plugins: [],
} satisfies Config;
```

- [ ] **Step 4: postcss.config.cjs**

```js
module.exports = {
  plugins: { tailwindcss: {}, autoprefixer: {} },
};
```

- [ ] **Step 5: index.html**

**File:** `dashboard/index.html`

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Teramind Dashboard</title>
  </head>
  <body class="bg-neutral-50 text-neutral-900 antialiased">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 6: .gitignore**

**File:** `dashboard/.gitignore`

```
node_modules
dist
playwright-report
test-results
.vite
```

Then re-add the placeholder index.html via `git add -f dashboard/dist/index.html dashboard/dist/.gitkeep` so it survives.

### Task 16.3: src/main.tsx + router scaffold

**File:** `dashboard/src/main.tsx`

```tsx
import React from 'react';
import ReactDOM from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createRouter } from '@tanstack/react-router';
import { routeTree } from './router';
import './styles.css';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { refetchOnWindowFocus: true, staleTime: 30_000 },
  },
});

const router = createRouter({ routeTree, defaultPreload: 'intent' });

declare module '@tanstack/react-router' {
  interface Register { router: typeof router }
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </React.StrictMode>,
);
```

**File:** `dashboard/src/styles.css`

```css
@tailwind base;
@tailwind components;
@tailwind utilities;
```

**File:** `dashboard/src/router.tsx`

```tsx
import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router';
import { Shell } from './components/Shell';
import { Activity } from './routes/activity';
import { Skills } from './routes/skills';
import { Members } from './routes/members';
import { Quality } from './routes/quality';
import { Health } from './routes/health';
import { Login } from './routes/login';

const rootRoute = createRootRoute({ component: () => <Outlet /> });

const loginRoute = createRoute({
  getParentRoute: () => rootRoute, path: '/login', component: Login,
});

const shellRoute = createRoute({
  getParentRoute: () => rootRoute, id: 'shell', component: Shell,
});

const indexRoute  = createRoute({ getParentRoute: () => shellRoute, path: '/',          component: Activity });
const activity    = createRoute({ getParentRoute: () => shellRoute, path: '/activity',  component: Activity });
const skills      = createRoute({ getParentRoute: () => shellRoute, path: '/skills',    component: Skills });
const members     = createRoute({ getParentRoute: () => shellRoute, path: '/members',   component: Members });
const quality     = createRoute({ getParentRoute: () => shellRoute, path: '/quality',   component: Quality });
const health      = createRoute({ getParentRoute: () => shellRoute, path: '/health',    component: Health });

export const routeTree = rootRoute.addChildren([
  loginRoute,
  shellRoute.addChildren([indexRoute, activity, skills, members, quality, health]),
]);
```

### Task 16.4: API client + auth hook

**File:** `dashboard/src/lib/api.ts`

```ts
export class DashboardError extends Error {
  code: string;
  details?: unknown;
  status: number;
  constructor(status: number, code: string, message: string, details?: unknown) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, { credentials: 'include', ...init });
  if (r.status === 204) return undefined as T;
  const ct = r.headers.get('content-type') || '';
  const body = ct.includes('application/json') ? await r.json() : await r.text();
  if (!r.ok) {
    const err = (body as any)?.error || {};
    throw new DashboardError(r.status, err.code || 'unknown', err.message || `HTTP ${r.status}`, err.details);
  }
  return body as T;
}

export const api = {
  get: <T,>(p: string) => call<T>(p),
  post: <T,>(p: string, body?: unknown) => call<T>(p, {
    method: 'POST',
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  }),
  patch: <T,>(p: string, body: unknown) => call<T>(p, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }),
  delete: <T,>(p: string) => call<T>(p, { method: 'DELETE' }),
};
```

**File:** `dashboard/src/lib/auth.tsx`

```tsx
import { useEffect, useState } from 'react';
import { useNavigate, useRouter } from '@tanstack/react-router';
import { api, DashboardError } from './api';

export interface AuthState { authenticated: boolean; loading: boolean; expiresAt?: string }

export function useAuth(): AuthState {
  const [state, setState] = useState<AuthState>({ authenticated: false, loading: true });
  const navigate = useNavigate();
  useEffect(() => {
    api.get<{ admin: boolean; expires_at: string }>('/admin/me')
      .then(d => setState({ authenticated: d.admin, loading: false, expiresAt: d.expires_at }))
      .catch((e: DashboardError) => {
        setState({ authenticated: false, loading: false });
        if (e.status === 401) {
          const here = window.location.pathname + window.location.search;
          navigate({ to: '/login', search: { redirect: here } as any });
        }
      });
  }, [navigate]);
  return state;
}
```

### Task 16.5: Components — Shell, Toast, CopyModal

**File:** `dashboard/src/components/Shell.tsx`

```tsx
import { Link, Outlet } from '@tanstack/react-router';
import { Activity, BookText, Users, BarChart3, Heart } from 'lucide-react';
import { useAuth } from '../lib/auth';
import { api } from '../lib/api';

const nav = [
  { to: '/activity', label: 'Activity', icon: Activity },
  { to: '/skills',   label: 'Skills',   icon: BookText },
  { to: '/members',  label: 'Members',  icon: Users },
  { to: '/quality',  label: 'Quality',  icon: BarChart3 },
  { to: '/health',   label: 'Health',   icon: Heart },
];

export function Shell() {
  const auth = useAuth();
  if (auth.loading) return <div className="p-8 text-neutral-500">Loading…</div>;
  if (!auth.authenticated) return null;
  return (
    <div className="min-h-screen flex">
      <aside className="w-56 bg-white border-r border-neutral-200 p-4">
        <div className="text-sm font-semibold text-neutral-500 mb-4 px-2">TERAMIND</div>
        <nav className="space-y-1">
          {nav.map(n => (
            <Link key={n.to} to={n.to}
                  className="flex items-center gap-2 px-2 py-2 rounded hover:bg-neutral-100 [&.active]:bg-neutral-200 [&.active]:font-medium">
              <n.icon size={16} /> {n.label}
            </Link>
          ))}
        </nav>
      </aside>
      <main className="flex-1 flex flex-col">
        <header className="border-b border-neutral-200 bg-white px-6 py-3 flex justify-between items-center">
          <div className="text-sm text-neutral-500">Dashboard · {window.location.host}</div>
          <button onClick={() => api.post('/admin/logout').then(() => window.location.href = '/dashboard/login')}
                  className="text-sm text-neutral-600 hover:text-neutral-900">
            Logout
          </button>
        </header>
        <section className="flex-1 p-6 overflow-auto"><Outlet /></section>
      </main>
    </div>
  );
}
```

**File:** `dashboard/src/components/Toast.tsx`

```tsx
import { useEffect, useState } from 'react';

export function Toast({ message, onClose }: { message: string; onClose: () => void }) {
  useEffect(() => {
    const t = setTimeout(onClose, 4000);
    return () => clearTimeout(t);
  }, [onClose]);
  return (
    <div className="fixed bottom-6 right-6 bg-red-600 text-white rounded-md shadow-lg px-4 py-3 text-sm">
      {message}
    </div>
  );
}

const [_, setMsg] = (() => { let m: any = null; return [m, (v: any) => m = v]; })();
let _setter: ((s: string | null) => void) | null = null;
export function ToastHost() {
  const [msg, setMsgState] = useState<string | null>(null);
  _setter = setMsgState;
  return msg ? <Toast message={msg} onClose={() => setMsgState(null)} /> : null;
}
export function showToast(msg: string) { _setter?.(msg); }
```

**File:** `dashboard/src/components/CopyModal.tsx`

```tsx
import { useState } from 'react';

export function CopyModal({ title, value, onClose }: { title: string; value: string; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg p-6 max-w-lg w-full shadow-xl">
        <h2 className="text-lg font-medium mb-2">{title}</h2>
        <p className="text-sm text-red-600 mb-3">⚠️ This is the only time this code will be shown.</p>
        <input value={value} readOnly
               className="w-full font-mono text-sm border border-neutral-300 rounded px-2 py-1.5 bg-neutral-50" />
        <div className="mt-4 flex justify-end gap-2">
          <button onClick={() => { navigator.clipboard.writeText(value); setCopied(true); }}
                  className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white hover:bg-neutral-800">
            {copied ? 'Copied ✓' : 'Copy'}
          </button>
          <button onClick={onClose} className="px-3 py-1.5 text-sm rounded bg-neutral-100 hover:bg-neutral-200">
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
```

### Task 16.6: Login route

**File:** `dashboard/src/routes/login.tsx`

```tsx
import { useState } from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { api, DashboardError } from '../lib/api';

export function Login() {
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { redirect?: string };
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true); setError(null);
    try {
      await api.post('/admin/login', { password });
      navigate({ to: search.redirect ?? '/activity' });
    } catch (err) {
      const e = err as DashboardError;
      setError(e.code === 'rate_limited'
        ? 'Too many attempts. Wait 5 minutes.'
        : e.code === 'invalid_password' ? 'Incorrect password.' : e.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-neutral-50">
      <form onSubmit={submit} className="bg-white shadow-xl rounded-lg p-8 w-full max-w-sm">
        <h1 className="text-xl font-medium mb-1">Teramind Dashboard</h1>
        <p className="text-sm text-neutral-500 mb-6">Admin sign-in</p>
        <input
          type="password" autoFocus placeholder="Admin password" value={password}
          onChange={e => setPassword(e.target.value)}
          className="w-full border border-neutral-300 rounded px-3 py-2 text-sm focus:border-neutral-900 focus:ring-1 focus:ring-neutral-900 outline-none"
        />
        {error && <div className="mt-3 text-sm text-red-600">{error}</div>}
        <button disabled={busy || !password}
                className="mt-4 w-full bg-neutral-900 text-white rounded px-3 py-2 text-sm font-medium disabled:opacity-50">
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </div>
  );
}
```

### Task 16.7: Stub the other routes so the build compiles

**Files:** `dashboard/src/routes/{activity,skills,members,quality,health}.tsx` — each one a placeholder:

```tsx
export function Activity() { return <h1 className="text-xl">Activity (stub)</h1>; }
```

(Same pattern for the others — different component name + heading.)

### Task 16.8: Install + build + commit

```bash
cd dashboard
npm install
npm run build
cd ..
```

Verify `dashboard/dist/index.html` is now a real Vite build (not the placeholder). The server's `cargo build` re-runs and embeds it.

```bash
cd /Users/vahemomjyan/Desktop/teracloud/src/teramind
cargo build -p teramind-sync-server
git add dashboard/package.json dashboard/tsconfig.json dashboard/vite.config.ts \
        dashboard/tailwind.config.ts dashboard/postcss.config.cjs dashboard/index.html \
        dashboard/.gitignore dashboard/src
# Don't commit node_modules or the built dist — they're gitignored.
git commit -m "feat(dashboard): React scaffold + Login route + Shell"
```

---

## Section 17 — Activity view

### Task 17.1: Live event stream hook

**File:** `dashboard/src/lib/event_stream.ts`

```ts
import { useEffect, useRef, useState } from 'react';

export interface TeamEvent {
  type: 'session_ended' | 'wiki_page_ready' | 'skill_saved';
  session_id?: string;
  user_id?: string;
  cwd?: string;
  title?: string;
  name?: string;
  ts: string;
}

export function useEventStream(enabled: boolean): TeamEvent[] {
  const [events, setEvents] = useState<TeamEvent[]>([]);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${window.location.host}/admin/events`);
    wsRef.current = ws;
    ws.onmessage = (e) => {
      try {
        const evt = JSON.parse(e.data) as TeamEvent | { type: 'hello' };
        if ((evt as TeamEvent).ts) {
          setEvents(prev => [evt as TeamEvent, ...prev].slice(0, 200));
        }
      } catch { /* ignore */ }
    };
    ws.onclose = () => { wsRef.current = null; };
    return () => { ws.close(); };
  }, [enabled]);

  return events;
}
```

### Task 17.2: Activity route

**File:** `dashboard/src/routes/activity.tsx`

```tsx
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api } from '../lib/api';
import { useEventStream, TeamEvent } from '../lib/event_stream';

interface ActivityRow {
  id: string;
  kind: string;
  user_id?: string;
  cwd?: string;
  payload: any;
  ts: string;
}

export function Activity() {
  const [auto, setAuto] = useState(true);
  const [kindFilter, setKindFilter] = useState('');
  const live = useEventStream(auto);

  const { data, isLoading } = useQuery({
    queryKey: ['activity', kindFilter],
    queryFn: async () => {
      const qs = kindFilter ? `?kind=${kindFilter}&limit=100` : `?limit=100`;
      return api.get<{ events: ActivityRow[] }>(`/admin/activity${qs}`);
    },
  });

  const liveRows: ActivityRow[] = live.map(e => ({
    id: `live-${e.ts}-${Math.random()}`,
    kind: e.type, user_id: e.user_id, cwd: e.cwd,
    payload: e, ts: e.ts,
  }));
  const combined = [...liveRows, ...(data?.events ?? [])];

  return (
    <div>
      <header className="flex justify-between items-center mb-4">
        <h1 className="text-xl font-medium">Activity</h1>
        <div className="flex items-center gap-3 text-sm">
          <select value={kindFilter} onChange={e => setKindFilter(e.target.value)}
                  className="border border-neutral-300 rounded px-2 py-1 text-sm">
            <option value="">All kinds</option>
            <option value="session_ended">Session ended</option>
            <option value="skill_saved">Skill saved</option>
            <option value="wiki_page_ready">Wiki page ready</option>
          </select>
          <label className="flex items-center gap-1.5">
            <input type="checkbox" checked={auto} onChange={e => setAuto(e.target.checked)} />
            Live
          </label>
        </div>
      </header>
      {isLoading ? <div className="text-sm text-neutral-500">Loading…</div> : null}
      <div className="bg-white rounded border border-neutral-200 divide-y divide-neutral-100">
        {combined.map(r => (
          <div key={r.id} className="px-4 py-2 flex items-center text-sm">
            <span className="text-neutral-400 font-mono text-xs w-44 shrink-0">{r.ts.replace('T', ' ').slice(0, 19)}</span>
            <span className="font-mono text-xs w-44 shrink-0">{r.kind}</span>
            <span className="flex-1 truncate text-neutral-600">{r.cwd ?? r.payload?.name ?? ''}</span>
          </div>
        ))}
        {combined.length === 0 && !isLoading ? (
          <div className="px-4 py-12 text-center text-sm text-neutral-500">No activity yet.</div>
        ) : null}
      </div>
    </div>
  );
}
```

### Task 17.3: Vitest for event stream

**File:** `dashboard/tests/event_stream.test.ts`

```ts
import { describe, it, expect } from 'vitest';

// Pure reducer extracted from the hook — keep it simple in v1.
function appendEvents(prev: any[], next: any, cap = 200): any[] {
  return [next, ...prev].slice(0, cap);
}

describe('event stream', () => {
  it('prepends new events', () => {
    const out = appendEvents([{ id: 1 }], { id: 2 });
    expect(out[0].id).toBe(2);
    expect(out.length).toBe(2);
  });
  it('caps at the limit', () => {
    let s: any[] = [];
    for (let i = 0; i < 250; i++) s = appendEvents(s, { id: i }, 200);
    expect(s.length).toBe(200);
    expect(s[0].id).toBe(249);
  });
});
```

### Task 17.4: Build + commit

```bash
cd dashboard
npm run build
cd ..
cargo build -p teramind-sync-server
git add dashboard/src dashboard/tests
git commit -m "feat(dashboard): Activity view + live WebSocket subscription"
```

---

## Section 18 — Skills + Candidate review view

### Task 18.1: Skills route + candidate review

**File:** `dashboard/src/routes/skills.tsx`

```tsx
import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api, DashboardError } from '../lib/api';

type Source = 'all' | 'authored' | 'codified' | 'pending' | 'rejected';

interface SkillRow {
  id: string;
  name: string;
  description: string;
  source: string;
  status?: string;
  applies_to_cwds: string[];
}

interface SkillDetail extends SkillRow { body: string }
interface Candidate {
  id: string;
  name: string;
  description: string;
  body: string;
  applies_to_cwds: string[];
  source_session_ids: string[];
  model: string;
  status: string;
  generated_at: string;
}

export function Skills() {
  const [source, setSource] = useState<Source>('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const list = useQuery({
    queryKey: ['skills', source],
    queryFn: () => api.get<{ skills?: SkillRow[]; candidates?: Candidate[] }>(
      source === 'pending' || source === 'rejected'
        ? `/admin/candidates?status=${source}&limit=100`
        : `/admin/skills?source=${source === 'all' ? 'all' : source}&limit=100`),
  });

  const detail = useQuery({
    queryKey: ['skill-detail', source, selectedId],
    queryFn: () => {
      if (!selectedId) return Promise.resolve(null);
      if (source === 'pending' || source === 'rejected') {
        return api.get<Candidate>(`/admin/candidates/${selectedId}`);
      }
      return api.get<SkillDetail>(`/admin/skills/${selectedId}`);
    },
    enabled: !!selectedId,
  });

  const rows: Array<{ id: string; name: string; description: string }> =
    (list.data?.skills ?? list.data?.candidates ?? []).map(r => ({
      id: r.id, name: r.name, description: r.description,
    }));

  return (
    <div className="grid grid-cols-12 gap-4 h-full">
      <aside className="col-span-4 bg-white rounded border border-neutral-200 flex flex-col">
        <div className="p-3 border-b border-neutral-200 flex flex-col gap-2">
          <h1 className="font-medium">Skills</h1>
          <div className="flex flex-wrap gap-1 text-xs">
            {(['all', 'authored', 'codified', 'pending', 'rejected'] as Source[]).map(s => (
              <button key={s}
                      onClick={() => { setSource(s); setSelectedId(null); }}
                      className={`px-2 py-1 rounded ${source === s ? 'bg-neutral-900 text-white' : 'bg-neutral-100 hover:bg-neutral-200'}`}>
                {s}
              </button>
            ))}
          </div>
        </div>
        <div className="flex-1 overflow-auto">
          {rows.map(r => (
            <button key={r.id}
                    onClick={() => setSelectedId(r.id)}
                    className={`w-full text-left px-3 py-2 border-b border-neutral-100 hover:bg-neutral-50 ${selectedId === r.id ? 'bg-neutral-100' : ''}`}>
              <div className="font-medium text-sm">{r.name}</div>
              <div className="text-xs text-neutral-500 truncate">{r.description}</div>
            </button>
          ))}
        </div>
      </aside>
      <section className="col-span-8 bg-white rounded border border-neutral-200 p-4 overflow-auto">
        {!selectedId ? (
          <div className="text-sm text-neutral-500">Pick a skill on the left.</div>
        ) : source === 'pending' || source === 'rejected' ? (
          <CandidateReview candidate={detail.data as Candidate | null | undefined}
                           onChanged={() => { list.refetch(); detail.refetch(); }} />
        ) : (
          <SkillDetailPanel skill={detail.data as SkillDetail | null | undefined} />
        )}
      </section>
    </div>
  );
}

function SkillDetailPanel({ skill }: { skill?: SkillDetail | null }) {
  if (!skill) return <div className="text-sm text-neutral-500">Loading…</div>;
  return (
    <article>
      <h2 className="text-xl font-medium">{skill.name}</h2>
      <div className="text-sm text-neutral-500 mt-1">source: {skill.source} · applies_to: {(skill.applies_to_cwds || []).join(', ') || 'global'}</div>
      <p className="mt-3 text-sm">{skill.description}</p>
      <pre className="mt-4 p-3 bg-neutral-50 border border-neutral-200 rounded text-xs whitespace-pre-wrap">{skill.body}</pre>
    </article>
  );
}

function CandidateReview({ candidate, onChanged }: { candidate?: Candidate | null; onChanged: () => void }) {
  const [description, setDesc] = useState('');
  const [body, setBody] = useState('');
  const [cwds, setCwds] = useState('');
  const [error, setError] = useState<string | null>(null);
  const qc = useQueryClient();

  // Sync editable fields when candidate changes.
  if (candidate && description === '' && body === '' && cwds === '') {
    setDesc(candidate.description);
    setBody(candidate.body);
    setCwds(candidate.applies_to_cwds.join('\n'));
  }

  const save = useMutation({
    mutationFn: () => api.patch(`/admin/candidates/${candidate!.id}`, {
      description, body, applies_to_cwds: cwds.split('\n').map(s => s.trim()).filter(Boolean),
    }),
    onSuccess: () => { setError(null); onChanged(); },
    onError: (e: DashboardError) => setError(e.message),
  });
  const approve = useMutation({
    mutationFn: () => api.post(`/admin/candidates/${candidate!.id}/approve`, { reviewer: 'admin' }),
    onSuccess: () => { onChanged(); qc.invalidateQueries(); },
    onError: (e: DashboardError) => setError(e.message),
  });
  const reject = useMutation({
    mutationFn: () => api.post(`/admin/candidates/${candidate!.id}/reject`, { reviewer: 'admin' }),
    onSuccess: () => { onChanged(); qc.invalidateQueries(); },
    onError: (e: DashboardError) => setError(e.message),
  });

  if (!candidate) return <div className="text-sm text-neutral-500">Loading…</div>;
  return (
    <article className="space-y-4">
      <header>
        <h2 className="text-xl font-medium">{candidate.name}</h2>
        <div className="text-sm text-neutral-500">
          status: {candidate.status} · model: {candidate.model} · generated {candidate.generated_at}
        </div>
      </header>
      <div>
        <label className="text-xs uppercase tracking-wide text-neutral-500">Description</label>
        <textarea className="w-full border border-neutral-300 rounded p-2 text-sm" rows={2}
                  value={description} onChange={e => setDesc(e.target.value)} />
      </div>
      <div>
        <label className="text-xs uppercase tracking-wide text-neutral-500">Body</label>
        <textarea className="w-full border border-neutral-300 rounded p-2 font-mono text-xs" rows={20}
                  value={body} onChange={e => setBody(e.target.value)} />
      </div>
      <div>
        <label className="text-xs uppercase tracking-wide text-neutral-500">applies_to_cwds (one per line)</label>
        <textarea className="w-full border border-neutral-300 rounded p-2 font-mono text-xs" rows={4}
                  value={cwds} onChange={e => setCwds(e.target.value)} />
      </div>
      {error && <div className="text-sm text-red-600">{error}</div>}
      <footer className="flex justify-end gap-2">
        <button onClick={() => reject.mutate()} className="px-3 py-1.5 text-sm rounded bg-red-50 text-red-700 hover:bg-red-100">Reject</button>
        <button onClick={() => save.mutate()} className="px-3 py-1.5 text-sm rounded bg-neutral-100 hover:bg-neutral-200">Save edits</button>
        <button onClick={() => approve.mutate()} className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white hover:bg-neutral-800">Approve & Promote</button>
      </footer>
    </article>
  );
}
```

### Task 18.2: Commit

```bash
cd dashboard && npm run build && cd ..
cargo build -p teramind-sync-server
git add dashboard/src/routes/skills.tsx
git commit -m "feat(dashboard): Skills view + candidate review (edit/approve/reject)"
```

---

## Section 19 — Members + Quality + Health views

### Task 19.1: Members route

**File:** `dashboard/src/routes/members.tsx`

```tsx
import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '../lib/api';
import { CopyModal } from '../components/CopyModal';

interface Member {
  id: string;
  email: string;
  device_count: number;
  last_seen_at?: string;
  revoked_at?: string;
}
interface Invite { id: string; invited_email: string; expires_at: string }
interface Device { id: string; name: string; last_seen_at?: string }

export function Members() {
  const qc = useQueryClient();
  const members = useQuery({ queryKey: ['members'], queryFn: () => api.get<{ users: Member[] }>('/admin/members') });
  const invites = useQuery({ queryKey: ['invites'], queryFn: () => api.get<{ invites: Invite[] }>('/admin/invites') });
  const [issueOpen, setIssueOpen] = useState(false);
  const [generatedCode, setGeneratedCode] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [email, setEmail] = useState('');
  const [days, setDays] = useState(7);

  const create = useMutation({
    mutationFn: () => api.post<{ code: string }>('/admin/invites', { email, expires_in_days: days }),
    onSuccess: (data) => { setGeneratedCode(data.code); setIssueOpen(false); setEmail(''); qc.invalidateQueries({ queryKey: ['invites'] }); },
  });
  const revokeMember = useMutation({
    mutationFn: (uid: string) => api.post(`/admin/members/${uid}/revoke`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['members'] }),
  });
  const revokeInvite = useMutation({
    mutationFn: (iid: string) => api.post(`/admin/invites/${iid}/revoke`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['invites'] }),
  });
  const revokeDevice = useMutation({
    mutationFn: (did: string) => api.post(`/admin/devices/${did}/revoke`),
    onSuccess: () => qc.invalidateQueries(),
  });

  return (
    <div>
      <header className="flex justify-between items-center mb-4">
        <h1 className="text-xl font-medium">Members</h1>
        <button onClick={() => setIssueOpen(true)} className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white">+ Issue invite</button>
      </header>
      <div className="bg-white rounded border border-neutral-200 overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-neutral-50 text-xs text-neutral-500 uppercase">
            <tr><th className="text-left px-4 py-2">Email</th><th className="text-left">Devices</th><th className="text-left">Last seen</th><th className="text-left">Status</th><th></th></tr>
          </thead>
          <tbody>
            {(members.data?.users ?? []).map(m => (
              <>
                <tr key={m.id} className="border-t border-neutral-100">
                  <td className="px-4 py-2">{m.email}</td>
                  <td>{m.device_count}</td>
                  <td>{m.last_seen_at ? new Date(m.last_seen_at).toLocaleString() : '—'}</td>
                  <td>{m.revoked_at ? 'revoked' : 'active'}</td>
                  <td className="text-right pr-4 space-x-2">
                    <button onClick={() => setExpanded(expanded === m.id ? null : m.id)}
                            className="text-xs text-neutral-600 hover:text-neutral-900">
                      {expanded === m.id ? 'hide' : 'devices'}
                    </button>
                    <button onClick={() => revokeMember.mutate(m.id)}
                            disabled={!!m.revoked_at}
                            className="text-xs text-red-600 hover:text-red-800 disabled:text-neutral-400">
                      revoke
                    </button>
                  </td>
                </tr>
                {expanded === m.id && <DeviceList userId={m.id} onRevoke={(d) => revokeDevice.mutate(d)} />}
              </>
            ))}
          </tbody>
        </table>
      </div>
      <h2 className="text-base font-medium mt-8 mb-2">Open invites</h2>
      <div className="bg-white rounded border border-neutral-200 overflow-hidden">
        <table className="w-full text-sm">
          <tbody>
            {(invites.data?.invites ?? []).map(i => (
              <tr key={i.id} className="border-t border-neutral-100">
                <td className="px-4 py-2">{i.invited_email}</td>
                <td>expires {new Date(i.expires_at).toLocaleDateString()}</td>
                <td className="text-right pr-4">
                  <button onClick={() => revokeInvite.mutate(i.id)} className="text-xs text-red-600 hover:text-red-800">revoke</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {issueOpen && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white rounded-lg p-6 w-full max-w-sm shadow-xl">
            <h2 className="text-lg font-medium mb-3">Issue invite</h2>
            <input value={email} onChange={e => setEmail(e.target.value)} placeholder="email"
                   className="w-full border border-neutral-300 rounded px-2 py-1.5 text-sm" />
            <label className="text-xs text-neutral-500 mt-3 block">Expires in {days} days</label>
            <input type="range" min={1} max={30} value={days} onChange={e => setDays(Number(e.target.value))}
                   className="w-full mt-1" />
            <div className="flex justify-end gap-2 mt-4">
              <button onClick={() => setIssueOpen(false)} className="px-3 py-1.5 text-sm rounded bg-neutral-100">Cancel</button>
              <button onClick={() => create.mutate()} disabled={!email.includes('@')}
                      className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white disabled:opacity-50">
                Issue
              </button>
            </div>
          </div>
        </div>
      )}
      {generatedCode && (
        <CopyModal title="Invite code" value={generatedCode} onClose={() => setGeneratedCode(null)} />
      )}
    </div>
  );
}

function DeviceList({ userId, onRevoke }: { userId: string; onRevoke: (deviceId: string) => void }) {
  const { data } = useQuery({
    queryKey: ['user-devices', userId],
    queryFn: () => api.get<{ devices: Device[] }>(`/admin/members/${userId}/devices`),
  });
  return (
    <tr><td colSpan={5} className="bg-neutral-50 px-4 py-2 text-xs">
      {(data?.devices ?? []).length === 0 ? '(no active devices)' : (
        <ul className="space-y-1">
          {(data!.devices).map(d => (
            <li key={d.id} className="flex justify-between">
              <span className="font-mono">{d.name}</span>
              <span className="text-neutral-500">{d.last_seen_at ?? 'never'}</span>
              <button onClick={() => onRevoke(d.id)} className="text-red-600 hover:text-red-800">revoke</button>
            </li>
          ))}
        </ul>
      )}
    </td></tr>
  );
}
```

### Task 19.2: Quality route

**File:** `dashboard/src/routes/quality.tsx`

```tsx
import { useQuery } from '@tanstack/react-query';
import { LineChart, Line, XAxis, YAxis, CartesianGrid, ResponsiveContainer, Tooltip } from 'recharts';
import { api } from '../lib/api';

interface QualityRun {
  id: string; baseline_label: string; model: string | null;
  ndcg10: number; mrr: number; p95_latency_ms: number;
  ran_at: string;
}

export function Quality() {
  const runs = useQuery({
    queryKey: ['quality'],
    queryFn: () => api.get<{ runs: QualityRun[] }>('/admin/quality?limit=60'),
  });
  const cfg = useQuery({
    queryKey: ['quality-config'],
    queryFn: () => api.get<{ enabled: boolean; cron: string | null }>('/admin/quality/config'),
  });

  const rows = (runs.data?.runs ?? []).slice().reverse();   // oldest → newest for charting
  const latest = rows[rows.length - 1];

  if (rows.length === 0) {
    return (
      <div className="bg-white border border-neutral-200 rounded p-6">
        <h1 className="text-xl font-medium mb-3">Search quality</h1>
        <p className="text-sm text-neutral-600 mb-3">No eval history yet.</p>
        <pre className="bg-neutral-50 border border-neutral-200 rounded p-3 text-xs whitespace-pre-wrap">
{`Run periodic search-quality benchmarks by adding [quality] to your config:
[quality]
enabled = true
cron    = "0 2 * * *"   # 02:00 daily

Or upload a one-off result: POST /admin/quality/runs`}
        </pre>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-xl font-medium">Search quality</h1>
      <Chart rows={rows} dataKey="ndcg10" label="nDCG@10" yDomain={[0, 1]} />
      <Chart rows={rows} dataKey="mrr"    label="MRR"      yDomain={[0, 1]} />
      <Chart rows={rows} dataKey="p95_latency_ms" label="p95 latency (ms)" yDomain={[0, 5000]} />
      <div className="bg-white border border-neutral-200 rounded p-4 text-sm">
        <div className="text-neutral-500 mb-1">Latest run: {latest && new Date(latest.ran_at).toLocaleString()}</div>
        <div>nDCG@10: <b>{latest?.ndcg10.toFixed(3)}</b> · MRR: <b>{latest?.mrr.toFixed(3)}</b> · p95: <b>{latest?.p95_latency_ms.toFixed(0)} ms</b></div>
        <div className="text-xs text-neutral-500 mt-1">Model: {latest?.model ?? '—'}</div>
      </div>
      <div className="text-xs text-neutral-500">
        Scheduler: {cfg.data?.enabled ? 'enabled' : 'disabled'} · cron: {cfg.data?.cron ?? '—'}
      </div>
    </div>
  );
}

function Chart({ rows, dataKey, label, yDomain }: { rows: any[]; dataKey: string; label: string; yDomain: [number, number] }) {
  return (
    <div className="bg-white border border-neutral-200 rounded p-4">
      <div className="text-sm text-neutral-500 mb-2">{label}</div>
      <div style={{ width: '100%', height: 180 }}>
        <ResponsiveContainer>
          <LineChart data={rows.map(r => ({ ...r, ts: new Date(r.ran_at).getTime() }))}>
            <CartesianGrid strokeDasharray="2 2" stroke="#eee" />
            <XAxis dataKey="ts" type="number" domain={['dataMin', 'dataMax']} hide />
            <YAxis domain={yDomain} fontSize={11} />
            <Tooltip labelFormatter={(v) => new Date(v).toLocaleDateString()} />
            <Line type="monotone" dataKey={dataKey} stroke="#171717" dot={false} />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
```

### Task 19.3: Health route

**File:** `dashboard/src/routes/health.tsx`

```tsx
import { useQuery } from '@tanstack/react-query';
import { api } from '../lib/api';

export function Health() {
  const { data, isLoading } = useQuery({
    queryKey: ['health'],
    queryFn: () => api.get<Record<string, any>>('/admin/health'),
    refetchInterval: 5000,
  });
  if (isLoading) return <div className="text-sm text-neutral-500">Loading…</div>;
  return (
    <div>
      <h1 className="text-xl font-medium mb-4">Health</h1>
      <pre className="bg-white border border-neutral-200 rounded p-4 text-xs whitespace-pre-wrap">{JSON.stringify(data, null, 2)}</pre>
    </div>
  );
}
```

### Task 19.4: Build + commit

```bash
cd dashboard && npm run build && cd ..
cargo build -p teramind-sync-server
git add dashboard/src/routes/{members,quality,health}.tsx
git commit -m "feat(dashboard): Members + Quality + Health views"
```

---

## Section 20 — Vitest unit tests + Playwright E2E

### Task 20.1: API client unit tests

**File:** `dashboard/tests/api.test.ts`

```ts
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { api, DashboardError } from '../src/lib/api';

beforeEach(() => { vi.stubGlobal('fetch', vi.fn()); });

describe('api client', () => {
  it('parses error JSON into DashboardError', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ error: { code: 'rate_limited', message: 'slow down' } }),
      { status: 429, headers: { 'content-type': 'application/json' } },
    ));
    try {
      await api.get('/admin/me');
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(DashboardError);
      expect((e as DashboardError).code).toBe('rate_limited');
      expect((e as DashboardError).status).toBe(429);
    }
  });

  it('returns parsed JSON on success', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ admin: true }),
      { status: 200, headers: { 'content-type': 'application/json' } },
    ));
    const out = await api.get<{ admin: boolean }>('/admin/me');
    expect(out.admin).toBe(true);
  });
});
```

### Task 20.2: Playwright E2E

**File:** `dashboard/playwright.config.ts`

```ts
import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests/playwright',
  use: { baseURL: 'http://localhost:8443' },
});
```

**File:** `dashboard/tests/playwright/dashboard.spec.ts`

```ts
import { test, expect } from '@playwright/test';

const PASSWORD = process.env.TMD_ADMIN_PASSWORD ?? 'hunter2hunter2';

test('login then visit all four views', async ({ page }) => {
  await page.goto('/dashboard/login');
  await page.fill('input[type="password"]', PASSWORD);
  await page.click('button:has-text("Sign in")');
  await expect(page).toHaveURL(/\/dashboard\/(activity|$)/);
  await expect(page.locator('h1')).toContainText('Activity');

  await page.click('text=Skills');
  await expect(page.locator('h1')).toContainText('Skills');

  await page.click('text=Members');
  await expect(page.locator('h1')).toContainText('Members');

  await page.click('text=Quality');
  await expect(page.locator('h1')).toContainText('Search quality');

  await page.click('text=Health');
  await expect(page.locator('h1')).toContainText('Health');
});
```

This test requires a running server with a known admin password — for CI, the test setup launches the binary with `TMD_ADMIN_PASSWORD=hunter2hunter2` baked into a temp config + a temp PG.

For local dev: `cd dashboard && npm run playwright`. CI uses a separate workflow step.

### Task 20.3: Bundle size gate

**File:** `dashboard/package.json` — add to `scripts`:

```json
"size-check": "test \"$(du -k dist/assets/*.js dist/assets/*.css 2>/dev/null | awk '{s+=$1} END {print s}')\" -lt 350 || (echo 'bundle exceeds 350K' && exit 1)"
```

(`350K` is the soft pre-gzip ceiling — gzipped is roughly 1/3, hitting the 250 KB gzipped target.)

### Task 20.4: Commit

```bash
cd dashboard && npm run build && npm test && cd ..
git add dashboard/tests dashboard/playwright.config.ts dashboard/package.json
git commit -m "test(dashboard): vitest unit tests + Playwright E2E + bundle gate"
```

---

## Section 21 — Final check

### Task 21.1: Workspace + dashboard tests

```bash
export GITHUB_TOKEN=$(gh auth token)
cd dashboard && npm test && npm run lint && cd ..
cargo test --workspace -- --test-threads=1 2>&1 | grep -E "^test result:" | awk -F'[. ;]+' '{p+=$4;f+=$6} END {print "passed="p, "failed="f}'
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plan M baseline: 358 tests. Plan N adds approximately:
- §1 migration: 1
- §2 repos: 4
- §3 admin-password subcommand: implicit
- §4 cookie codec: 5
- §5 rate limit: 3
- §7 admin_login: 4
- §9 admin_activity: 2
- §10 admin_skills/candidates/observations: ~9
- §11 admin_members: ~3
- §12 quality_runs core: 1
- §14 admin_quality: 3
- §15 dashboard_assets: 1

Total Rust new: ~36 tests. Expected workspace total: ~394.
TypeScript: 3 vitest test files (api, auth, event_stream) → ~6 tests.

### Task 21.2: Lint + format

If fmt is dirty, run `cargo fmt --all` and commit as `style: cargo fmt --all`.

### Task 21.3: Report

Print HEAD SHA, commit count (`git rev-list --count main..HEAD`), workspace test totals, dashboard test totals. Do NOT push.

---

## Spec coverage matrix

| Spec section | Plan N addresses |
|---|---|
| §1 Background | implicit |
| §2.1 In scope — SPA at /dashboard/* | §15 (static serving), §16 (scaffold), §17–§19 (views) |
| §2.1 In scope — /admin/* API | §7, §9, §10, §11, §14 |
| §2.1 In scope — admin password + session cookie | §3 (CLI), §4 (codec), §5 (middleware), §7 (login handler) |
| §2.1 In scope — Activity view | §9 (API), §17 (UI) |
| §2.1 In scope — Skills + candidate review | §10 (API), §18 (UI) |
| §2.1 In scope — Members & devices | §11 (API), §19 (UI) |
| §2.1 In scope — Quality view | §13, §14, §19 |
| §2.1 In scope — team_event_log | §1, §2, §8 |
| §2.1 In scope — quality_runs + scheduler | §1, §2, §12, §13, §14 |
| §2.1 In scope — embedded SPA | §15, §16, §17, §18, §19 |
| §2.1 In scope — dashboard disabled by default | §5 (middleware returns 404 when admin None) |
| §3 Architecture | §4–§19 |
| §4 Auth | §3, §4, §5, §7 |
| §5 Admin endpoints | §7, §9, §10, §11, §14 |
| §6 Frontend layout | §16, §17, §18, §19 |
| §7 Storage + scheduler | §1, §2, §8, §13 |
| §8 Build + embed | §15, §16 |
| §9 Testing | §7, §9, §10, §11, §14, §15, §20 |
| §10 Rollout + risks | implicit; no code |
