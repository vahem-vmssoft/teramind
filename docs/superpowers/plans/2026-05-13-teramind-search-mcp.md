# Teramind Search + MCP (Plan C) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make captured traces and skills retrievable through all four surfaces named in the spec — `teramind search` CLI, `mcp__teramind__search`/`recall`/`save_skill` MCP tools, `/teramind:search` and `/teramind:recall` slash commands, and the auto-recall digest injected by the `SessionStart` hook. End state: a developer can run `teramind search "stack overflow in serializer"` and get ranked hits; the model inside Claude can call `mcp__teramind__search` mid-session and receive structured prior context; and Claude session open auto-injects a relevant context digest.

**Architecture:** Search lives inside `teramindd` as one service queried over IPC. It runs two ranking strategies in parallel (Postgres `tsvector` via `ts_rank_cd` for natural language; `pg_trgm` similarity for code excerpts) and blends them with recency-decay and same-project boost. A new `teramind-mcp` binary (built on the `rmcp` SDK) translates MCP stdio tool calls into IPC `Request::Search`/`Recall`/`SaveSkill` and returns typed `Hit` results. The CLI surface is a thin wrapper on the same IPC. Grep fallback over the JSONL shadow log activates when Postgres is unreachable. Auto-recall is a daemon endpoint the `SessionStart` hook calls to print a markdown digest to stdout for Claude to inject.

**Tech Stack:** Rust stable, `tokio`, `sqlx` (already pinned), `rmcp` (MCP Rust SDK for the MCP stdio server), existing `teramind-core` / `teramind-ipc` / `teramind-db` / `teramindd` / `teramind-hook` from Plans A+B.

**Spec reference:** `docs/superpowers/specs/2026-05-13-teramind-core-design.md`, Section 6 (Search and retrieval) — covers ranking blend, MCP tool contract, auto-recall, grep fallback. Plan A laid the schema (`traces_fts` MV, pg_trgm GIN indices, `Hit` enum), Plan B wired capture; Plan C now makes the captured data queryable.

**State prerequisites (must hold before starting):**
- Plan A complete on `main` (66 commits, daemon + DB + IPC + CLI all live).
- Plan B complete on `main` (35 commits, hook + plugin install + capture E2E green).
- `traces_fts` materialized view exists with `to_tsvector('english', user_prompt || assistant_text || thinking || string_agg(tool_calls.output) || string_agg(file_diffs.unified_diff))` and indexes `traces_fts_document` (GIN), `traces_fts_turn_id` (UNIQUE).
- `Hit` enum already defined in `teramind-core/src/types/hit.rs` with variants `Turn`/`ToolCall`/`FileDiff`/`Skill`.

---

## File Structure

```
teramind/
├── Cargo.toml                                    [+ add crates/teramind-mcp to members; + rmcp workspace dep]
├── crates/
│   ├── teramind-core/                            [+ search request/response types]
│   │   └── src/types/
│   │       ├── search.rs                        [SearchRequest, RecallRequest, SearchResults, SkillRef]
│   │       └── mod.rs                           [+ pub mod search; pub use search::*;]
│   ├── teramind-ipc/                             [+ extend Request/Response enums with Search/Recall/SaveSkill]
│   │   └── src/proto.rs                         [extended]
│   ├── teramind-db/                              [+ SearchRepo with tsvector + pg_trgm + ranking SQL]
│   │   └── src/repos/
│   │       ├── search.rs                        [NEW: SearchRepo]
│   │       └── mod.rs                           [+ pub mod search;]
│   ├── teramindd/                                [+ search service + IPC handlers]
│   │   └── src/services/
│   │       ├── search.rs                        [NEW: ranking blend + hydration + grep fallback dispatch]
│   │       ├── grep_fallback.rs                 [NEW: tokio::process grep over JSONL]
│   │       └── mod.rs                           [+ pub mod search; pub mod grep_fallback;]
│   ├── teramind-mcp/                             [NEW CRATE: MCP stdio server using rmcp]
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs                          [stdio MCP server entry]
│   │   │   ├── server.rs                        [rmcp ServerHandler impl]
│   │   │   └── tools/
│   │   │       ├── mod.rs
│   │   │       ├── search.rs
│   │   │       ├── recall.rs
│   │   │       └── save_skill.rs
│   │   └── tests/
│   │       └── tools.rs                         [unit + IPC fake]
│   ├── teramind-hook/                            [+ SessionStart auto-recall digest printing]
│   │   └── src/
│   │       ├── auto_recall.rs                   [NEW: queries daemon for recall digest, prints to stdout]
│   │       └── main.rs                          [modified: route SessionStart → also run auto_recall]
│   └── teramind/                                 [CLI: + `teramind search` subcommand]
│       └── src/commands/
│           └── search.rs                        [NEW]
├── plugins/
│   └── claude/
│       ├── commands/                             [NEW: slash command definitions]
│       │   ├── search.md
│       │   └── recall.md
│       └── .mcp.json                            [NEW: declares teramind-mcp as an MCP server]
└── docs/
    └── runbooks/
        └── claude-search-manual-smoke.md        [L4 procedure for verifying search surfaces with real Claude]
```

**Why these boundaries:** `teramind-mcp` is its own binary so its dependency footprint is the MCP SDK + IPC client, with no DB/sqlx pollution. `SearchRepo` keeps SQL out of the daemon service. The `grep_fallback.rs` module is dispatch-from-disk and has no DB dep, so it can be tested without Postgres. Slash commands and `.mcp.json` extend the plugin template (Plan B), letting `teramind claude install` continue to work uniformly with its placeholder substitution.

**End-state architecture (search request flow):**

```
CLI:          teramind search "X"      ──► IPC::Request::Search { query: "X", limit: 10 }
MCP tool:     mcp__teramind__search    ──► (via teramind-mcp) IPC::Request::Search
Slash cmd:    /teramind:search X       ──► (via teramind-mcp tool registration) IPC::Request::Search
Auto-recall:  SessionStart hook        ──► (via teramind-hook auto_recall) IPC::Request::AutoRecall { cwd }
                                                   │
                                                   ▼
                                         daemon::services::search
                                                   │
                              ┌────────────────────┼─────────────────────┐
                              ▼                    ▼                     ▼
                     SearchRepo::fts        SearchRepo::trgm    (Postgres unreachable?)
                     (tsvector)             (pg_trgm)                    │
                              │                    │                     ▼
                              └────────┬───────────┘            grep_fallback::run
                                       ▼                       (tokio process grep)
                              ranking blend
                              + recency decay
                              + project boost
                                       │
                                       ▼
                              hydrate hits (load surrounding turn/diff/skill)
                                       │
                                       ▼
                              SearchResults { hits, degraded: bool, took_ms }
```

---

## Section 1 — Workspace: add `teramind-mcp` crate and `rmcp` workspace dep

### Task 1: Register `teramind-mcp` and pin `rmcp`

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/teramind-mcp/Cargo.toml`
- Create: `crates/teramind-mcp/src/main.rs`

- [ ] **Step 1: Update workspace `members` and add `rmcp` workspace dep**

In root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/teramind-core",
    "crates/teramind-ipc",
    "crates/teramind-db",
    "crates/teramindd",
    "crates/teramind",
    "crates/teramind-hook",
    "crates/teramind-mcp",
]
```

In `[workspace.dependencies]`, add (alphabetical placement is fine):

```toml
rmcp = { version = "0.2", features = ["server", "transport-io"] }
```

(`rmcp` is the official MCP Rust SDK; the `server` feature is the server-side bits, `transport-io` provides the stdio transport. Verify exact version on crates.io before pinning; the spec authorizes use of `rmcp` if mature enough — early-2026 stable versions are in the 0.x line.)

- [ ] **Step 2: Write `crates/teramind-mcp/Cargo.toml`**

```toml
[package]
name = "teramind-mcp"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "teramind-mcp"
path = "src/main.rs"

[dependencies]
teramind-core = { path = "../teramind-core" }
teramind-ipc  = { path = "../teramind-ipc" }
rmcp        = { workspace = true }
anyhow      = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tokio       = { workspace = true }
tracing     = { workspace = true }
async-trait = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Stub `crates/teramind-mcp/src/main.rs`**

```rust
fn main() {
    eprintln!("teramind-mcp: not yet implemented");
    std::process::exit(2);
}
```

- [ ] **Step 4: Verify workspace resolves**

Run: `cargo metadata --format-version=1 --no-deps 2>&1 | head -5`
Expected: clean JSON, members count is now 7.

If `rmcp` fails to resolve from crates.io (e.g. version mismatch), fall back to the latest published 0.x version. Update both the workspace dep and Task 7's `use rmcp::...` paths accordingly.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/teramind-mcp/
git commit -m "chore: register teramind-mcp crate + pin rmcp workspace dep"
```

---

## Section 2 — Search types in `teramind-core`

### Task 2: `SearchRequest`, `RecallRequest`, `SaveSkillRequest`, `SearchResults`, `SkillRef`

**Files:**
- Create: `crates/teramind-core/src/types/search.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Add module declaration**

Append to `crates/teramind-core/src/types/mod.rs`:

```rust
pub mod search;
pub use search::*;
```

- [ ] **Step 2: Write `crates/teramind-core/src/types/search.rs`**

```rust
use crate::ids::{SessionId, SkillId};
use crate::types::Hit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "SearchRequest::default_limit")]
    pub limit: u32,
}

impl SearchRequest {
    fn default_limit() -> u32 { 10 }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RecallRequest {
    pub cwd: Option<String>,
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub stack_traces: Vec<String>,
    #[serde(default = "RecallRequest::default_limit")]
    pub limit: u32,
}

impl RecallRequest {
    fn default_limit() -> u32 { 10 }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoRecallRequest {
    pub cwd: String,
    #[serde(default = "AutoRecallRequest::default_limit")]
    pub limit: u32,
}

impl AutoRecallRequest {
    fn default_limit() -> u32 { 5 }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveSkillRequest {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub source_session_ids: Vec<SessionId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    #[serde(default)]
    pub degraded: bool,
    pub took_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRef {
    pub id: SkillId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_default_limit_when_missing() {
        let r: SearchRequest = serde_json::from_str(r#"{"query":"x"}"#).unwrap();
        assert_eq!(r.limit, 10);
    }

    #[test]
    fn search_results_roundtrips() {
        let r = SearchResults { hits: vec![], degraded: false, took_ms: 42 };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(r, serde_json::from_str(&j).unwrap());
    }

    #[test]
    fn auto_recall_request_default_limit() {
        let r: AutoRecallRequest = serde_json::from_str(r#"{"cwd":"/w"}"#).unwrap();
        assert_eq!(r.limit, 5);
        assert_eq!(r.cwd, "/w");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p teramind-core types::search`
Expected: 3 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): search request/response types"
```

---

## Section 3 — IPC protocol extension

### Task 3: Extend `Request` and `Response` with Search/Recall/SaveSkill/AutoRecall

**Files:**
- Modify: `crates/teramind-ipc/src/proto.rs`

- [ ] **Step 1: Update `Request` enum**

Replace the `Request` enum body with:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Status,
    Ping,
    Shutdown,
    Search(teramind_core::types::SearchRequest),
    Recall(teramind_core::types::RecallRequest),
    AutoRecall(teramind_core::types::AutoRecallRequest),
    SaveSkill(teramind_core::types::SaveSkillRequest),
}
```

- [ ] **Step 2: Update `Response` enum** to add `SearchResults` and `SkillRef` variants:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Pong,
    Status(StatusReport),
    Error(String),
    SearchResults(teramind_core::types::SearchResults),
    SkillRef(teramind_core::types::SkillRef),
    AutoRecallDigest { markdown: String, degraded: bool },
}
```

- [ ] **Step 3: Run existing IPC tests to confirm no regressions**

Run: `cargo test -p teramind-ipc`
Expected: all existing tests still PASS (the new variants are additive).

- [ ] **Step 4: Add a new roundtrip test for Search request**

Append to `crates/teramind-ipc/src/proto.rs` inside the existing `mod tests` block:

```rust
#[test]
fn search_request_roundtrips() {
    let env = Envelope {
        id: uuid::Uuid::new_v4(),
        payload: Payload::Request(Request::Search(
            teramind_core::types::SearchRequest { query: "stack overflow".into(), limit: 5 }
        )),
    };
    let j = serde_json::to_string(&env).unwrap();
    let back: Envelope = serde_json::from_str(&j).unwrap();
    assert_eq!(env, back);
}

#[test]
fn search_results_response_roundtrips() {
    let env = Envelope {
        id: uuid::Uuid::new_v4(),
        payload: Payload::Response(Response::SearchResults(
            teramind_core::types::SearchResults { hits: vec![], degraded: false, took_ms: 8 }
        )),
    };
    let j = serde_json::to_string(&env).unwrap();
    let back: Envelope = serde_json::from_str(&j).unwrap();
    assert_eq!(env, back);
}
```

Run: `cargo test -p teramind-ipc proto::tests::search_request_roundtrips proto::tests::search_results_response_roundtrips`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-ipc/src/proto.rs
git commit -m "feat(ipc): extend Request/Response with Search/Recall/AutoRecall/SaveSkill"
```

---

## Section 4 — `SearchRepo` (raw SQL queries)

### Task 4: Scaffold `SearchRepo` and FTS query

**Files:**
- Create: `crates/teramind-db/src/repos/search.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Add `pub mod search; pub use search::SearchRepo;` to `repos/mod.rs`.**

- [ ] **Step 2: Write `crates/teramind-db/src/repos/search.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::SessionId;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct SearchRepo {
    pool: DbPool,
}

/// One ranked candidate from a single ranking strategy.
/// The daemon `search` service blends these across strategies.
#[derive(Debug, Clone)]
pub struct RankedTurn {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    pub ordinal: i32,
    pub ts: OffsetDateTime,
    pub project_id: Option<Uuid>,
    pub fts_score: f32,
    pub trgm_score: f32,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RankedDiff {
    pub diff_id: Uuid,
    pub session_id: Uuid,
    pub rel_path: String,
    pub ts: OffsetDateTime,
    pub project_id: Option<Uuid>,
    pub trgm_score: f32,
    pub pre_excerpt: String,
    pub post_excerpt: String,
}

#[derive(Debug, Clone)]
pub struct RankedSkill {
    pub skill_id: Uuid,
    pub name: String,
    pub body: String,
    pub trgm_score: f32,
}

impl SearchRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    /// Run the tsvector-based full-text query against `traces_fts`.
    /// Returns up to `limit` turns ranked by `ts_rank_cd`.
    pub async fn fts_turns(&self, query: &str, limit: u32) -> Result<Vec<RankedTurn>> {
        let rows: Vec<(Uuid, Uuid, i32, OffsetDateTime, Option<Uuid>, f32, Option<String>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT
                f.turn_id, f.session_id, f.ordinal, f.ts,
                s.project_id,
                ts_rank_cd(f.document, plainto_tsquery('english', $1))::float4 AS fts_score,
                t.user_prompt, t.assistant_text
            FROM traces_fts f
            JOIN turns    t ON t.id = f.turn_id
            JOIN sessions s ON s.id = f.session_id
            WHERE f.document @@ plainto_tsquery('english', $1)
            ORDER BY fts_score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(turn_id, session_id, ordinal, ts, project_id, fts, prompt, text)| {
            RankedTurn { turn_id, session_id, ordinal, ts, project_id, fts_score: fts, trgm_score: 0.0,
                         user_prompt: prompt, assistant_text: text }
        }).collect())
    }

    /// Trigram-similarity query against file_diffs.pre_excerpt and post_excerpt.
    pub async fn trgm_diffs(&self, query: &str, limit: u32) -> Result<Vec<RankedDiff>> {
        let rows: Vec<(Uuid, Uuid, String, OffsetDateTime, Option<Uuid>, f32, String, String)> = sqlx::query_as(
            r#"
            SELECT
                fd.id, fd.session_id, fd.rel_path, fd.captured_at,
                s.project_id,
                GREATEST(similarity(fd.pre_excerpt, $1), similarity(fd.post_excerpt, $1))::float4 AS trgm_score,
                fd.pre_excerpt, fd.post_excerpt
            FROM file_diffs fd
            JOIN sessions s ON s.id = fd.session_id
            WHERE fd.pre_excerpt %% $1 OR fd.post_excerpt %% $1
            ORDER BY trgm_score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(diff_id, session_id, rel_path, ts, project_id, trgm, pre, post)| {
            RankedDiff { diff_id, session_id, rel_path, ts, project_id, trgm_score: trgm, pre_excerpt: pre, post_excerpt: post }
        }).collect())
    }

    /// Trigram-similarity query against skills.
    pub async fn trgm_skills(&self, query: &str, limit: u32) -> Result<Vec<RankedSkill>> {
        let rows: Vec<(Uuid, String, String, f32)> = sqlx::query_as(
            r#"
            SELECT id, name, body,
                   GREATEST(similarity(name, $1), similarity(body, $1))::float4 AS trgm_score
            FROM skills
            WHERE name %% $1 OR body %% $1
            ORDER BY trgm_score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(skill_id, name, body, trgm)| {
            RankedSkill { skill_id, name, body, trgm_score: trgm }
        }).collect())
    }

    /// Most-recent turns for a session (used for auto-recall hydration).
    pub async fn recent_turns_in_project(&self, project_id: Option<Uuid>, cwd: &str, limit: u32) -> Result<Vec<RankedTurn>> {
        // If project_id is known, match by it; else fall back to cwd prefix.
        let rows: Vec<(Uuid, Uuid, i32, OffsetDateTime, Option<Uuid>, Option<String>, Option<String>)> = match project_id {
            Some(pid) => sqlx::query_as(
                r#"
                SELECT t.id, t.session_id, t.ordinal, t.started_at, s.project_id, t.user_prompt, t.assistant_text
                FROM turns t
                JOIN sessions s ON s.id = t.session_id
                WHERE s.project_id = $1
                ORDER BY t.started_at DESC
                LIMIT $2
                "#,
            ).bind(pid).bind(limit as i64).fetch_all(self.pool.pg()).await?,
            None => sqlx::query_as(
                r#"
                SELECT t.id, t.session_id, t.ordinal, t.started_at, s.project_id, t.user_prompt, t.assistant_text
                FROM turns t
                JOIN sessions s ON s.id = t.session_id
                WHERE s.cwd = $1
                ORDER BY t.started_at DESC
                LIMIT $2
                "#,
            ).bind(cwd).bind(limit as i64).fetch_all(self.pool.pg()).await?,
        };
        Ok(rows.into_iter().map(|(turn_id, session_id, ordinal, ts, project_id, prompt, text)| {
            RankedTurn { turn_id, session_id, ordinal, ts, project_id, fts_score: 0.0, trgm_score: 0.0,
                         user_prompt: prompt, assistant_text: text }
        }).collect())
    }

    pub async fn upsert_skill(&self, req: &teramind_core::types::SaveSkillRequest) -> Result<teramind_core::types::SkillRef> {
        let row: (Uuid, String) = sqlx::query_as(
            r#"
            INSERT INTO skills (name, description, body, source, source_session_ids)
            VALUES ($1, $2, $3, 'authored', $4)
            ON CONFLICT (name) DO UPDATE SET
              description = EXCLUDED.description,
              body        = EXCLUDED.body,
              source_session_ids = EXCLUDED.source_session_ids,
              updated_at  = now()
            RETURNING id, name
            "#,
        )
        .bind(&req.name)
        .bind(&req.description)
        .bind(&req.body)
        .bind(req.source_session_ids.iter().map(|s| s.0).collect::<Vec<Uuid>>())
        .fetch_one(self.pool.pg()).await?;
        Ok(teramind_core::types::SkillRef {
            id: teramind_core::ids::SkillId(row.0),
            name: row.1,
        })
    }
}
```

Note: the existing `SkillRepo::upsert_authored` from Plan A only handles a 3-arg variant. The new `SearchRepo::upsert_skill` extends it with `source_session_ids`. The two can coexist; `SkillRepo` is unchanged.

- [ ] **Step 3: Compile-check**

Run: `cargo check -p teramind-db`
Expected: clean.

- [ ] **Step 4: Add an integration test using the shared fixture**

Append to `crates/teramind-db/tests/repos.rs`:

```rust
#[tokio::test]
async fn search_repo_fts_finds_matching_turn() {
    let f = Fixture::new().await;
    // Seed: one session, one turn with assistant_text containing "redis cluster"
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = teramind_db::repos::SessionRepo::new(f.pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = teramind_db::repos::TraceRepo::new(f.pool.clone());
    let turn = trace.upsert_turn(sid, 0, now, Some("how to debug redis cluster failover")).await.unwrap();
    trace.finalize_turn(turn, now, Some("the redis cluster needs sentinel quorum"), None, None, None, None).await.unwrap();
    // Refresh the materialized view so it picks up the new turn.
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(f.pool.pg()).await.unwrap();

    let repo = teramind_db::repos::SearchRepo::new(f.pool.clone());
    let hits = repo.fts_turns("redis cluster", 10).await.unwrap();
    assert_eq!(hits.len(), 1, "expected exactly one fts hit");
    assert_eq!(hits[0].turn_id, turn.0);
    assert!(hits[0].fts_score > 0.0);

    f.shutdown().await;
}

#[tokio::test]
async fn search_repo_trgm_finds_matching_diff() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = teramind_db::repos::SessionRepo::new(f.pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let diffs = teramind_db::repos::DiffRepo::new(f.pool.clone());
    diffs.insert(teramind_db::repos::diff::NewFileDiff {
        turn_id: None, session_id: sid,
        file_path: "/w/parser.rs", rel_path: "parser.rs",
        attribution: teramind_core::types::file_diff::Attribution::Agent,
        language: Some("rust"),
        pre_excerpt: "fn parse_jwt_payload(token: &str) -> Result<Claims>",
        post_excerpt: "fn parse_jwt_payload(token: &str, leeway: u64) -> Result<Claims>",
        unified_diff: "--- a\n+++ b\n", pre_hash: [0u8; 32], post_hash: [1u8; 32],
        byte_size: 50, captured_at: now,
    }).await.unwrap();

    let repo = teramind_db::repos::SearchRepo::new(f.pool.clone());
    let hits = repo.trgm_diffs("parse_jwt_payload", 10).await.unwrap();
    assert_eq!(hits.len(), 1, "expected one trgm hit");
    assert!(hits[0].trgm_score > 0.3);

    f.shutdown().await;
}
```

Run: `cargo test -p teramind-db --test repos search_repo_fts_finds_matching_turn search_repo_trgm_finds_matching_diff -- --nocapture`
Expected: 2 PASS.

**Note:** The FTS test calls `REFRESH MATERIALIZED VIEW traces_fts` explicitly. Plan A's daemon refreshes this MV every 30s in background; in tests we trigger it inline so the test isn't timing-dependent.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/ crates/teramind-db/tests/repos.rs
git commit -m "feat(db): SearchRepo with fts_turns, trgm_diffs, trgm_skills, recent_turns_in_project, upsert_skill"
```

---

## Section 5 — Daemon search service: ranking + hydration

### Task 5: `search::blend_and_rank` core logic (unit-testable, no DB)

**Files:**
- Create: `crates/teramindd/src/services/search.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod search;` to `crates/teramindd/src/services/mod.rs`.**

- [ ] **Step 2: Write the skeleton of `search.rs` with the pure ranking blend (no DB calls yet)**

```rust
use teramind_core::types::Hit;
use teramind_core::ids::{FileDiffId, SessionId, SkillId, ToolCallId, TurnId};
use teramind_db::repos::search::{RankedDiff, RankedSkill, RankedTurn};
use std::time::Instant;
use time::OffsetDateTime;
use uuid::Uuid;

/// Configuration for the ranking blend. Spec §6.1 default weights.
#[derive(Debug, Clone, Copy)]
pub struct BlendWeights {
    pub fts: f32,
    pub trgm: f32,
    pub recency: f32,
    pub project: f32,
}

impl Default for BlendWeights {
    fn default() -> Self {
        Self { fts: 0.6, trgm: 0.4, recency: 0.2, project: 0.3 }
    }
}

/// Apply weighted blend, recency decay, project boost.
/// Returns a final score for ranking.
pub fn final_score(fts: f32, trgm: f32, ts: OffsetDateTime, weights: BlendWeights, same_project: bool) -> f32 {
    let recency_decay = recency_factor(ts);
    let project_boost = if same_project { 1.0 } else { 0.0 };
    weights.fts * fts
        + weights.trgm * trgm
        + weights.recency * recency_decay
        + weights.project * project_boost
}

/// exp(-age_days / 90) where age_days is days since the row's timestamp.
fn recency_factor(ts: OffsetDateTime) -> f32 {
    let age = OffsetDateTime::now_utc() - ts;
    let days = age.whole_seconds() as f32 / 86_400.0;
    (-days / 90.0).exp()
}

/// Take per-strategy candidates and produce ranked `Hit`s.
/// `same_project_id` is the caller's project_id if known (e.g. for `Recall` requests).
pub fn rank_and_hydrate(
    fts_turns: Vec<RankedTurn>,
    trgm_diffs: Vec<RankedDiff>,
    trgm_skills: Vec<RankedSkill>,
    weights: BlendWeights,
    same_project_id: Option<Uuid>,
    limit: u32,
) -> Vec<Hit> {
    // Index FTS hits by turn_id and merge trgm scores on collision (a turn could match both strategies).
    // For Plan C, keep it simple: combine by turn_id when FTS and trgm both fire on the same turn's content.
    // A diff's score isn't merged into its turn; diffs are surfaced as their own Hit::FileDiff variant.
    use std::collections::HashMap;
    let mut by_turn: HashMap<Uuid, RankedTurn> = HashMap::new();
    for t in fts_turns.into_iter() {
        by_turn.insert(t.turn_id, t);
    }
    let mut hits: Vec<(f32, Hit)> = Vec::new();
    for t in by_turn.into_values() {
        let same_p = same_project_id.map(|p| Some(p) == t.project_id).unwrap_or(false);
        let score = final_score(t.fts_score, t.trgm_score, t.ts, weights, same_p);
        let snippet = build_snippet(&t.user_prompt, &t.assistant_text);
        hits.push((score, Hit::Turn {
            turn_id: TurnId(t.turn_id),
            session_id: SessionId(t.session_id),
            ordinal: t.ordinal,
            snippet,
            score,
            ts: t.ts,
        }));
    }
    for d in trgm_diffs {
        let same_p = same_project_id.map(|p| Some(p) == d.project_id).unwrap_or(false);
        let score = final_score(0.0, d.trgm_score, d.ts, weights, same_p);
        let snippet = if d.post_excerpt.is_empty() { d.pre_excerpt.clone() } else { d.post_excerpt.clone() };
        hits.push((score, Hit::FileDiff {
            diff_id: FileDiffId(d.diff_id),
            rel_path: d.rel_path,
            hunk_snippet: snippet,
            score,
            ts: d.ts,
        }));
    }
    for s in trgm_skills {
        let score = final_score(0.0, s.trgm_score, OffsetDateTime::now_utc(), weights, false);
        hits.push((score, Hit::Skill {
            skill_id: SkillId(s.skill_id),
            name: s.name,
            body_snippet: truncate(&s.body, 200),
            score,
        }));
    }
    // Sort by score desc, take top `limit`.
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit as usize);
    hits.into_iter().map(|(_, h)| h).collect()
}

fn build_snippet(prompt: &Option<String>, text: &Option<String>) -> String {
    let mut out = String::new();
    if let Some(p) = prompt { out.push_str(&truncate(p, 120)); }
    if let Some(t) = text { if !out.is_empty() { out.push_str(" · "); } out.push_str(&truncate(t, 200)); }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() }
    else { let mut out: String = s.chars().take(n).collect(); out.push('…'); out }
}

/// Outcome of a `do_search` invocation: hits + how it was served.
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub degraded: bool,
    pub took_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn recency_factor_recent_is_near_1() {
        let r = recency_factor(OffsetDateTime::now_utc());
        assert!(r > 0.999, "expected ~1.0, got {r}");
    }

    #[test]
    fn recency_factor_90_days_old_is_near_exp_neg_1() {
        let r = recency_factor(OffsetDateTime::now_utc() - time::Duration::days(90));
        assert!((r - (-1.0f32).exp()).abs() < 0.01, "expected ~0.368, got {r}");
    }

    #[test]
    fn final_score_blends_with_recency_and_project_boost() {
        let weights = BlendWeights::default();
        let ts = OffsetDateTime::now_utc();
        let s1 = final_score(1.0, 1.0, ts, weights, true);
        let s2 = final_score(1.0, 1.0, ts, weights, false);
        // Project boost should make s1 > s2 by `weights.project` = 0.3.
        assert!((s1 - s2 - 0.3).abs() < 0.001);
    }

    #[test]
    fn rank_and_hydrate_orders_by_blended_score() {
        let now = OffsetDateTime::now_utc();
        let rank_a = RankedTurn {
            turn_id: uuid::Uuid::new_v4(), session_id: uuid::Uuid::new_v4(),
            ordinal: 0, ts: now, project_id: None,
            fts_score: 0.9, trgm_score: 0.0,
            user_prompt: Some("A".into()), assistant_text: None,
        };
        let rank_b = RankedTurn {
            turn_id: uuid::Uuid::new_v4(), session_id: uuid::Uuid::new_v4(),
            ordinal: 0, ts: now, project_id: None,
            fts_score: 0.5, trgm_score: 0.0,
            user_prompt: Some("B".into()), assistant_text: None,
        };
        let hits = rank_and_hydrate(vec![rank_a.clone(), rank_b.clone()], vec![], vec![], BlendWeights::default(), None, 10);
        assert_eq!(hits.len(), 2);
        // The first hit must be the one with the higher fts_score (rank_a).
        match &hits[0] {
            Hit::Turn { turn_id, .. } => assert_eq!(turn_id.0, rank_a.turn_id),
            other => panic!("expected Turn, got {other:?}"),
        }
    }
}
```

- [ ] **Step 3: Run the unit tests**

Run: `cargo test -p teramindd services::search::tests`
Expected: 4 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/search.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): search::final_score, rank_and_hydrate (pure ranking logic)"
```

---

### Task 6: `search::do_search` and `do_recall` (DB-backed)

**Files:**
- Modify: `crates/teramindd/src/services/search.rs`

- [ ] **Step 1: Add the DB-driven entry points to `search.rs`** (append, before the test module):

```rust
use teramind_db::repos::SearchRepo;
use teramind_core::types::{SearchRequest, RecallRequest, AutoRecallRequest};

pub async fn do_search(repo: &SearchRepo, req: &SearchRequest) -> Result<SearchOutcome, teramind_db::DbError> {
    let started = Instant::now();
    let (fts_res, trgm_diffs, trgm_skills) = tokio::try_join!(
        repo.fts_turns(&req.query, req.limit),
        repo.trgm_diffs(&req.query, req.limit),
        repo.trgm_skills(&req.query, req.limit),
    )?;
    let hits = rank_and_hydrate(fts_res, trgm_diffs, trgm_skills, BlendWeights::default(), None, req.limit);
    Ok(SearchOutcome { hits, degraded: false, took_ms: started.elapsed().as_millis() as u32 })
}

pub async fn do_recall(repo: &SearchRepo, req: &RecallRequest) -> Result<SearchOutcome, teramind_db::DbError> {
    let started = Instant::now();
    // Recall uses the structured filters as a query string when free-text fields are present.
    // For Plan C v1, we run FTS over symbols joined by spaces (stack_traces handled similarly)
    // and trgm over file_paths. Future plans can refine.
    let symbol_query = req.symbols.join(" ");
    let stacktrace_query = req.stack_traces.join(" ");
    let path_query = req.file_paths.join(" ");

    let (fts_sym, fts_st, trgm_paths) = tokio::try_join!(
        async {
            if symbol_query.is_empty() { Ok::<_, teramind_db::DbError>(vec![]) }
            else { repo.fts_turns(&symbol_query, req.limit).await }
        },
        async {
            if stacktrace_query.is_empty() { Ok::<_, teramind_db::DbError>(vec![]) }
            else { repo.fts_turns(&stacktrace_query, req.limit).await }
        },
        async {
            if path_query.is_empty() { Ok::<_, teramind_db::DbError>(vec![]) }
            else { repo.trgm_diffs(&path_query, req.limit).await }
        },
    )?;
    let merged: Vec<_> = fts_sym.into_iter().chain(fts_st.into_iter()).collect();
    let hits = rank_and_hydrate(merged, trgm_paths, vec![], BlendWeights::default(), None, req.limit);
    Ok(SearchOutcome { hits, degraded: false, took_ms: started.elapsed().as_millis() as u32 })
}

/// Build the auto-recall digest. Returns markdown text.
pub async fn do_auto_recall(repo: &SearchRepo, req: &AutoRecallRequest) -> Result<String, teramind_db::DbError> {
    let recent = repo.recent_turns_in_project(None, &req.cwd, req.limit).await?;
    if recent.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::new();
    out.push_str("## Recent Teramind context\n\n");
    for t in recent {
        let prompt_snippet = t.user_prompt.as_deref().unwrap_or("(no prompt)");
        let text_snippet = t.assistant_text.as_deref().unwrap_or("");
        out.push_str(&format!("- **{}**: {} · {}\n",
            t.ts.date(),
            truncate(prompt_snippet, 80),
            truncate(text_snippet, 120),
        ));
    }
    Ok(out)
}
```

- [ ] **Step 2: Add integration test**

Create `crates/teramindd/tests/search_e2e.rs`:

```rust
use teramind_core::types::SearchRequest;
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, SearchRepo};
use teramindd::services::search;
use tempfile::tempdir;

#[tokio::test]
async fn do_search_finds_seeded_turn_via_fts() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace.upsert_turn(sid, 0, now, Some("how to debug postgres replication lag")).await.unwrap();
    trace.finalize_turn(turn, now, Some("the replication lag means the standby is behind"), None, None, None, None).await.unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await.unwrap();

    let repo = SearchRepo::new(pool.clone());
    let req = SearchRequest { query: "replication lag".into(), limit: 10 };
    let out = search::do_search(&repo, &req).await.unwrap();
    assert!(out.hits.len() >= 1);
    assert!(!out.degraded);

    sup.shutdown().await.unwrap();
}
```

Run: `cargo test -p teramindd --test search_e2e do_search_finds_seeded_turn_via_fts -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/search.rs crates/teramindd/tests/search_e2e.rs
git commit -m "feat(daemon): do_search / do_recall / do_auto_recall against Postgres"
```

---

## Section 6 — Grep fallback

### Task 7: `grep_fallback::run` over JSONL

**Files:**
- Create: `crates/teramindd/src/services/grep_fallback.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod grep_fallback;` to `services/mod.rs`.**

- [ ] **Step 2: Write `grep_fallback.rs`**

```rust
use std::path::Path;
use tokio::process::Command;
use teramind_core::types::Hit;
use teramind_core::ids::{ClientEventId, SessionId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use uuid::Uuid;

/// Run `grep -rIEn` over the JSONL shadow log directory, parse hits into `Hit::Turn`
/// where the matching line was a `UserPrompt` or `AssistantTurn` event.
///
/// This is the degraded-mode search path when Postgres is unreachable.
pub async fn run(jsonl_dir: &Path, query: &str, limit: u32) -> std::io::Result<Vec<Hit>> {
    if !jsonl_dir.exists() {
        return Ok(vec![]);
    }
    let output = Command::new("grep")
        .arg("-rIEn")
        .arg("--include=*.jsonl")
        .arg(query)
        .arg(jsonl_dir)
        .output().await?;

    if !output.status.success() && output.status.code() != Some(1) {
        // grep exits 1 on no match; >1 is an error.
        return Err(std::io::Error::new(std::io::ErrorKind::Other,
            format!("grep failed: status={:?}", output.status)));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut hits: Vec<Hit> = Vec::new();
    for line in stdout.lines().take(limit as usize * 4) {
        // Each grep line is "path:lineno:body".
        let (_path, rest) = match line.split_once(':') { Some(p) => p, None => continue };
        let (_lineno, body) = match rest.split_once(':') { Some(p) => p, None => continue };
        let env: EventEnvelope = match serde_json::from_str(body) { Ok(e) => e, Err(_) => continue };
        match env.event {
            IngestEvent::UserPrompt { session_id, turn_ordinal, prompt } => {
                hits.push(Hit::Turn {
                    turn_id: TurnId(Uuid::nil()), // unknown in grep fallback
                    session_id, ordinal: turn_ordinal,
                    snippet: truncate_grep(&prompt, 200),
                    score: 0.5,
                    ts: env.ts,
                });
            }
            IngestEvent::AssistantTurn { turn_id, assistant_text, .. } => {
                hits.push(Hit::Turn {
                    turn_id,
                    session_id: SessionId(Uuid::nil()),
                    ordinal: -1,
                    snippet: truncate_grep(&assistant_text, 200),
                    score: 0.5,
                    ts: env.ts,
                });
            }
            _ => {}
        }
        if hits.len() >= limit as usize { break; }
    }
    let _ = ClientEventId::new(); // silence unused-import warnings (ClientEventId only used to assert envelope type pulls them in)
    Ok(hits)
}

fn truncate_grep(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::io::Write;

    #[tokio::test]
    async fn grep_finds_matching_user_prompt_line() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("2026-05-13.jsonl");
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(),
                turn_ordinal: 0,
                prompt: "stack overflow in serializer.rs:142".into(),
            },
        };
        let line = serde_json::to_vec(&env).unwrap();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&line).unwrap();
        writeln!(f).unwrap();

        let hits = run(tmp.path(), "serializer", 10).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one grep hit");
        match &hits[0] {
            Hit::Turn { snippet, .. } => assert!(snippet.contains("serializer")),
            other => panic!("expected Turn, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn grep_returns_empty_for_missing_dir() {
        let hits = run(std::path::Path::new("/nonexistent/teramind/raw"), "anything", 10).await.unwrap();
        assert!(hits.is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p teramindd services::grep_fallback::tests`
Expected: 2 PASS (the first requires `grep` on PATH — universally available on macOS / Linux / Windows WSL).

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/grep_fallback.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): grep fallback over JSONL shadow log"
```

---

### Task 8: Wire grep fallback into `do_search` on Postgres errors

**Files:**
- Modify: `crates/teramindd/src/services/search.rs`

- [ ] **Step 1: Introduce a `do_search_with_fallback` function**

Append to `search.rs`:

```rust
use std::path::Path;
use crate::services::grep_fallback;

/// Higher-level entry point: try PG, fall back to grep on error.
pub async fn do_search_with_fallback(
    repo: &SearchRepo,
    jsonl_dir: &Path,
    req: &SearchRequest,
) -> SearchOutcome {
    match do_search(repo, req).await {
        Ok(o) => o,
        Err(_) => {
            let started = Instant::now();
            let hits = grep_fallback::run(jsonl_dir, &req.query, req.limit).await.unwrap_or_default();
            SearchOutcome {
                hits,
                degraded: true,
                took_ms: started.elapsed().as_millis() as u32,
            }
        }
    }
}
```

- [ ] **Step 2: Add an integration test that kills Postgres mid-flight and asserts grep degraded path triggers**

Append to `crates/teramindd/tests/search_e2e.rs`:

```rust
#[tokio::test]
async fn do_search_falls_back_to_grep_when_pg_dies() {
    use std::io::Write;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    // Seed a JSONL line that grep can find when PG is gone.
    let jsonl_dir = tmp.path().join("raw"); std::fs::create_dir_all(&jsonl_dir).unwrap();
    let path = jsonl_dir.join("2026-05-13.jsonl");
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: time::OffsetDateTime::now_utc(),
        event: IngestEvent::UserPrompt {
            session_id: SessionId::new(), turn_ordinal: 0, prompt: "fallback works for grep".into(),
        },
    };
    let body = serde_json::to_vec(&env).unwrap();
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&body).unwrap();
    writeln!(f).unwrap();

    // Shut down Postgres explicitly.
    sup.shutdown().await.unwrap();
    // pool is now dangling; do_search will error.

    let repo = SearchRepo::new(pool.clone());
    let out = teramindd::services::search::do_search_with_fallback(
        &repo, &jsonl_dir, &SearchRequest { query: "fallback".into(), limit: 10 }
    ).await;
    assert!(out.degraded, "expected degraded result");
    assert!(!out.hits.is_empty(), "expected grep hit to come through");
}
```

Run: `cargo test -p teramindd --test search_e2e do_search_falls_back_to_grep_when_pg_dies`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/search.rs crates/teramindd/tests/search_e2e.rs
git commit -m "feat(daemon): do_search_with_fallback wires grep fallback on PG error"
```

---

## Section 7 — Daemon IPC handlers (Search / Recall / SaveSkill / AutoRecall)

### Task 9: Route Search / Recall / SaveSkill / AutoRecall in `DaemonIpcHandler::handle_request`

**Files:**
- Modify: `crates/teramindd/src/services/ipc_server.rs`
- Modify: `crates/teramindd/src/app.rs`

The `DaemonIpcHandler` struct already exists from Plan A. We extend it with a `SearchRepo` reference and an `Arc<PathBuf>` for the JSONL dir, then add handlers.

- [ ] **Step 1: Extend `DaemonIpcHandler` struct**

Edit `crates/teramindd/src/services/ipc_server.rs`. Update the struct:

```rust
pub struct DaemonIpcHandler {
    pub ingest: Arc<IngestService>,
    pub stats: Arc<IngestStats>,
    pub started: Instant,
    pub last_pg_bytes: std::sync::atomic::AtomicI64,
    pub last_jsonl_bytes: std::sync::atomic::AtomicI64,
    pub search_repo: teramind_db::repos::SearchRepo,
    pub jsonl_dir: std::path::PathBuf,
}
```

- [ ] **Step 2: Update `handle_request` to route the new variants**

Replace the `match req` body with:

```rust
async fn handle_request(&self, req: Request) -> Response {
    match req {
        Request::Status => Response::Status(StatusReport {
            uptime_seconds: self.started.elapsed().as_secs(),
            pg_connected: true,
            ingest_queue_depth: self.stats.queue_depth.load(Ordering::Relaxed) as u32,
            ingest_drops_total: self.stats.drops.load(Ordering::Relaxed),
            last_storage_pg_bytes: self.last_pg_bytes.load(Ordering::Relaxed),
            last_storage_jsonl_bytes: self.last_jsonl_bytes.load(Ordering::Relaxed),
        }),
        Request::Ping => Response::Pong,
        Request::Shutdown => Response::Ok,
        Request::Search(r) => {
            let out = crate::services::search::do_search_with_fallback(&self.search_repo, &self.jsonl_dir, &r).await;
            Response::SearchResults(teramind_core::types::SearchResults {
                hits: out.hits, degraded: out.degraded, took_ms: out.took_ms,
            })
        }
        Request::Recall(r) => {
            match crate::services::search::do_recall(&self.search_repo, &r).await {
                Ok(out) => Response::SearchResults(teramind_core::types::SearchResults {
                    hits: out.hits, degraded: out.degraded, took_ms: out.took_ms,
                }),
                Err(e) => Response::Error(format!("recall failed: {e}")),
            }
        }
        Request::AutoRecall(r) => {
            match crate::services::search::do_auto_recall(&self.search_repo, &r).await {
                Ok(md) => Response::AutoRecallDigest { markdown: md, degraded: false },
                Err(e) => Response::AutoRecallDigest { markdown: String::new(), degraded: true },
            }
        }
        Request::SaveSkill(r) => {
            match self.search_repo.upsert_skill(&r).await {
                Ok(s) => Response::SkillRef(s),
                Err(e) => Response::Error(format!("save_skill failed: {e}")),
            }
        }
    }
}
```

The `let _ = e;` warnings on the AutoRecall error branch should be silenced by using `_e` or `match ... { Ok(md) => ..., Err(_) => ... }`. The above form has an unused `e` in `Err(e)` for AutoRecall — change it to `Err(_)` to silence the warning.

- [ ] **Step 3: Update `App::run` to construct the handler with the new fields**

Edit `crates/teramindd/src/app.rs`. Locate the `let handler = Arc::new(DaemonIpcHandler { ... });` line and add the two new fields:

```rust
let handler = Arc::new(DaemonIpcHandler {
    ingest: ingest.clone(),
    stats: stats.clone(),
    started: Instant::now(),
    last_pg_bytes: 0.into(),
    last_jsonl_bytes: 0.into(),
    search_repo: teramind_db::repos::SearchRepo::new(pool.clone()),
    jsonl_dir: paths.raw_dir.clone(),
});
```

- [ ] **Step 4: Update existing test that constructs `DaemonIpcHandler`**

The `crates/teramindd/tests/ipc_status.rs` test from Plan A constructs `DaemonIpcHandler` literally. It must now include the two new fields. Edit it to add `search_repo` and `jsonl_dir`:

```rust
let handler = Arc::new(DaemonIpcHandler {
    ingest: Arc::new(svc),
    stats: stats.clone(),
    started: std::time::Instant::now(),
    last_pg_bytes: 0.into(),
    last_jsonl_bytes: 0.into(),
    search_repo: teramind_db::repos::SearchRepo::new(pool.clone()),
    jsonl_dir: tmp.path().join("raw"),
});
```

Same change to `crates/teramind-hook/tests/happy_path.rs` (which also constructs `DaemonIpcHandler` in two places).

- [ ] **Step 5: Run daemon tests to confirm no regression**

Run: `cargo test -p teramindd && cargo test -p teramind-hook --test happy_path`
Expected: PASS.

- [ ] **Step 6: Add an integration test for the IPC search round-trip**

Append to `crates/teramindd/tests/search_e2e.rs`:

```rust
#[tokio::test]
async fn ipc_search_request_returns_search_results() {
    use std::sync::Arc;
    use teramind_core::redact::Redactor;
    use teramind_ipc::client::{IpcClient, StreamClient};
    use teramind_ipc::proto::{Request, Response};
    use teramind_ipc::transport::{connect, listen};
    use teramindd::services::ingest::{IngestService, IngestStats, IngestDeps};
    use teramindd::services::jsonl_writer::JsonlWriter;
    use teramindd::services::session_manager::SessionManager;
    use teramindd::services::ipc_server::{DaemonIpcHandler, run_accept_loop};
    use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo, SearchRepo};

    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    // Seed
    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace.upsert_turn(sid, 0, now, Some("kafka consumer lag")).await.unwrap();
    trace.finalize_turn(turn, now, Some("the kafka consumer was behind"), None, None, None, None).await.unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let svc = IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(), sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()), session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()), diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(), dead_letter_dir: tmp.path().join("dl"),
    });
    let handler = Arc::new(DaemonIpcHandler {
        ingest: Arc::new(svc), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        search_repo: SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    let stream = connect(&sock).await.unwrap();
    let mut client = StreamClient::new(stream);
    let r = client.request(Request::Search(teramind_core::types::SearchRequest {
        query: "kafka".into(), limit: 10,
    })).await.unwrap();
    match r {
        Response::SearchResults(sr) => {
            assert!(!sr.hits.is_empty(), "expected at least one hit");
            assert!(!sr.degraded);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    sup.shutdown().await.unwrap();
}
```

Run: `cargo test -p teramindd --test search_e2e ipc_search_request_returns_search_results`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/teramindd/src/services/ipc_server.rs crates/teramindd/src/app.rs \
        crates/teramindd/tests/ipc_status.rs crates/teramindd/tests/search_e2e.rs \
        crates/teramind-hook/tests/happy_path.rs
git commit -m "feat(daemon): IPC handlers for Search/Recall/AutoRecall/SaveSkill"
```

---

## Section 8 — `teramind search` CLI subcommand

### Task 10: CLI search command

**Files:**
- Modify: `crates/teramind/src/cli.rs`
- Modify: `crates/teramind/src/main.rs`
- Create: `crates/teramind/src/commands/search.rs`
- Modify: `crates/teramind/src/commands/mod.rs`

- [ ] **Step 1: Add the `Search` subcommand to the CLI**

Edit `crates/teramind/src/cli.rs`. Append to `Command` enum:

```rust
/// Search prior traces and skills.
Search {
    /// The query text.
    query: String,
    /// Maximum hits to return.
    #[arg(short, long, default_value = "10")]
    limit: u32,
    /// Output as JSON instead of pretty text.
    #[arg(long)]
    json: bool,
    /// Force the grep fallback path.
    #[arg(long)]
    grep: bool,
},
```

- [ ] **Step 2: Wire into `main.rs`**

```rust
Command::Search { query, limit, json, grep } =>
    commands::search::run(query, limit, json, grep).await,
```

- [ ] **Step 3: Add to `commands/mod.rs`**: `pub mod search;`.

- [ ] **Step 4: Write `crates/teramind/src/commands/search.rs`**

```rust
use crate::ipc;
use teramind_core::types::{Hit, SearchRequest};
use teramind_ipc::proto::{Request, Response};

pub async fn run(query: String, limit: u32, json: bool, _grep: bool) -> anyhow::Result<()> {
    // _grep is reserved for future "force grep" wiring. v1 always lets the daemon decide.
    let resp = ipc::request(Request::Search(SearchRequest { query, limit }), 10_000).await?;
    let results = match resp {
        Response::SearchResults(s) => s,
        Response::Error(e) => { eprintln!("error: {e}"); return Ok(()); }
        other => { eprintln!("unexpected: {other:?}"); return Ok(()); }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }
    if results.degraded {
        eprintln!("(degraded: Postgres unreachable, served from JSONL via grep)");
    }
    eprintln!("({} hits in {} ms)", results.hits.len(), results.took_ms);
    for (i, h) in results.hits.iter().enumerate() {
        match h {
            Hit::Turn { session_id, ordinal, snippet, score, ts, .. } =>
                println!("{i:3}. [turn]    {ts}  session={session_id}#{ordinal}  score={score:.3}\n      {snippet}"),
            Hit::ToolCall { name, input_snippet, output_snippet, score, ts, .. } =>
                println!("{i:3}. [tool {name}]  {ts}  score={score:.3}\n      in:  {input_snippet}\n      out: {output_snippet}"),
            Hit::FileDiff { rel_path, hunk_snippet, score, ts, .. } =>
                println!("{i:3}. [diff]    {ts}  {rel_path}  score={score:.3}\n      {hunk_snippet}"),
            Hit::Skill { name, body_snippet, score, .. } =>
                println!("{i:3}. [skill]   {name}  score={score:.3}\n      {body_snippet}"),
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Build**

Run: `cargo build -p teramind-cli`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind/src/cli.rs crates/teramind/src/main.rs crates/teramind/src/commands/
git commit -m "feat(cli): teramind search subcommand"
```

---

## Section 9 — `teramind-mcp` binary (MCP stdio server using rmcp)

### Task 11: `teramind-mcp::server` — rmcp server scaffold

**Files:**
- Create: `crates/teramind-mcp/src/server.rs`
- Replace: `crates/teramind-mcp/src/main.rs`
- Create: `crates/teramind-mcp/src/tools/mod.rs`
- Create: `crates/teramind-mcp/src/tools/search.rs`
- Create: `crates/teramind-mcp/src/tools/recall.rs`
- Create: `crates/teramind-mcp/src/tools/save_skill.rs`

**Important: `rmcp` API compatibility.** The exact API of `rmcp` 0.2 is below. If the published version differs at implementation time, adapt the code minimally (the conceptual shape — `ServerHandler` trait, `tool` macro, stdio transport — is stable across 0.x minors).

- [ ] **Step 1: Write `server.rs`**

```rust
use anyhow::Result;
use rmcp::{
    handler::server::tool::{Parameters, ToolRouter},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo, Implementation},
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::Deserialize;
use std::sync::Arc;
use teramind_ipc::{client::{IpcClient, StreamClient}, proto::{Request, Response}, transport::{connect, default_socket_path}};

#[derive(Clone)]
pub struct TeramindMcpServer {
    tool_router: ToolRouter<Self>,
}

impl TeramindMcpServer {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    async fn ipc_request(&self, req: Request) -> Result<Response, McpError> {
        let path = default_socket_path();
        let stream = connect(&path).await
            .map_err(|e| McpError::internal_error(format!("connect daemon: {e}"), None))?;
        let mut client = StreamClient::new(stream);
        client.request(req).await
            .map_err(|e| McpError::internal_error(format!("ipc: {e}"), None))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}
fn default_limit() -> u32 { 10 }

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct RecallArgs {
    pub cwd: Option<String>,
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub stack_traces: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SaveSkillArgs {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[tool_router]
impl TeramindMcpServer {
    #[tool(description = "Search prior Claude sessions and skills by free text.")]
    async fn search(&self, Parameters(args): Parameters<SearchArgs>) -> Result<CallToolResult, McpError> {
        let req = Request::Search(teramind_core::types::SearchRequest {
            query: args.query, limit: args.limit,
        });
        let resp = self.ipc_request(req).await?;
        let body = match resp {
            Response::SearchResults(s) => serde_json::to_string_pretty(&s).unwrap_or_default(),
            Response::Error(e) => return Err(McpError::internal_error(e, None)),
            other => return Err(McpError::internal_error(format!("unexpected: {other:?}"), None)),
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Structured recall: filter prior context by cwd, files, symbols, or stack traces.")]
    async fn recall(&self, Parameters(args): Parameters<RecallArgs>) -> Result<CallToolResult, McpError> {
        let req = Request::Recall(teramind_core::types::RecallRequest {
            cwd: args.cwd, file_paths: args.file_paths, symbols: args.symbols,
            stack_traces: args.stack_traces, limit: args.limit,
        });
        let resp = self.ipc_request(req).await?;
        let body = match resp {
            Response::SearchResults(s) => serde_json::to_string_pretty(&s).unwrap_or_default(),
            Response::Error(e) => return Err(McpError::internal_error(e, None)),
            other => return Err(McpError::internal_error(format!("unexpected: {other:?}"), None)),
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    #[tool(description = "Save a user-authored skill into Teramind so future sessions can recall it.")]
    async fn save_skill(&self, Parameters(args): Parameters<SaveSkillArgs>) -> Result<CallToolResult, McpError> {
        let req = Request::SaveSkill(teramind_core::types::SaveSkillRequest {
            name: args.name, description: args.description, body: args.body,
            source_session_ids: vec![],
        });
        let resp = self.ipc_request(req).await?;
        let body = match resp {
            Response::SkillRef(s) => serde_json::to_string_pretty(&s).unwrap_or_default(),
            Response::Error(e) => return Err(McpError::internal_error(e, None)),
            other => return Err(McpError::internal_error(format!("unexpected: {other:?}"), None)),
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }
}

#[tool_handler]
impl ServerHandler for TeramindMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Teramind: search and recall prior Claude session traces.".into()),
            server_info: Implementation::from_build_env(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
```

Note: the `schemars` dep is needed for `JsonSchema` derives. Add `schemars = "0.8"` to teramind-mcp `Cargo.toml`:

```toml
schemars = "0.8"
```

If `rmcp 0.2` has a different surface (e.g. the `tool_router` macro is named differently), adapt to the actual API. The general shape (ServerHandler trait + a tool decorator + a stdio main) is stable.

- [ ] **Step 2: Write `crates/teramind-mcp/src/main.rs`**

```rust
use teramind_mcp::server::TeramindMcpServer;
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let service = TeramindMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

- [ ] **Step 3: Write `crates/teramind-mcp/src/lib.rs`** (so `tests/` can import `server`):

```rust
//! Teramind MCP stdio server.

pub mod server;
```

Add `[lib]` to `Cargo.toml`:

```toml
[lib]
name = "teramind_mcp"
path = "src/lib.rs"
```

- [ ] **Step 4: Build**

Run: `cargo build -p teramind-mcp`
Expected: clean.

If compilation fails because of rmcp API surface differences, capture the exact error and adjust the trait/struct/macro usage in `server.rs` to the actual published API. The conceptual shape (3 tools + ServerHandler) doesn't change.

- [ ] **Step 5: Smoke (manual, optional)**

```bash
# Start the daemon
teramind start

# Send an MCP initialize request manually
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}' | ./target/debug/teramind-mcp
```

Expected: JSON-RPC reply with capabilities. (Not asserted as a test — Section 12 integration test covers the live path.)

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-mcp/
git commit -m "feat(mcp): teramind-mcp stdio server with search/recall/save_skill tools"
```

---

## Section 10 — Claude plugin: slash commands + MCP registration

### Task 12: `/teramind:search` and `/teramind:recall` slash commands

**Files:**
- Create: `plugins/claude/commands/search.md`
- Create: `plugins/claude/commands/recall.md`

Claude Code slash commands are markdown files under `<plugin>/commands/`. The file name (minus `.md`) becomes the command name. The body of the file is rendered as a system prompt that nudges Claude to call the appropriate MCP tool.

- [ ] **Step 1: Write `plugins/claude/commands/search.md`**

```markdown
---
description: Search prior Teramind traces and skills.
argument-hint: "<query>"
---

The user wants you to search prior Claude sessions captured by Teramind. Call the `mcp__teramind__search` tool with the user's query: $ARGUMENTS. Show the top hits in a concise list.
```

- [ ] **Step 2: Write `plugins/claude/commands/recall.md`**

```markdown
---
description: Recall prior Teramind context relevant to the current session.
argument-hint: "[symbols or file paths]"
---

The user wants you to recall prior Teramind context. Call `mcp__teramind__recall` with the current cwd and any arguments the user provided ($ARGUMENTS) as file_paths or symbols (your judgment). Summarize the most relevant hits.
```

- [ ] **Step 3: Commit**

```bash
git add plugins/claude/commands/
git commit -m "feat(plugin): /teramind:search and /teramind:recall slash commands"
```

---

### Task 13: `.mcp.json` registering `teramind-mcp` as an MCP server

**Files:**
- Create: `plugins/claude/.mcp.json`

Claude Code reads a `.mcp.json` file in a plugin to register MCP servers.

- [ ] **Step 1: Write `plugins/claude/.mcp.json`**

```json
{
  "mcpServers": {
    "teramind": {
      "command": "@TERAMIND_MCP_BIN@",
      "args": [],
      "type": "stdio"
    }
  }
}
```

The `@TERAMIND_MCP_BIN@` placeholder is substituted by `teramind claude install` (Task 15).

- [ ] **Step 2: Commit**

```bash
git add plugins/claude/.mcp.json
git commit -m "feat(plugin): .mcp.json registers teramind-mcp as an MCP server"
```

---

## Section 11 — `teramind claude install`: ship MCP binary path + slash commands

### Task 14: Locate `teramind-mcp` and substitute placeholder

**Files:**
- Modify: `crates/teramind/src/commands/claude_install.rs`

The existing installer (Plan B) already walks the template directory and substitutes `@TERAMIND_PLUGIN_DIR@` and `@TERAMIND_HOOK_BIN@`. We extend it to locate `teramind-mcp` and substitute `@TERAMIND_MCP_BIN@`.

- [ ] **Step 1: Add a `which_teramind_mcp` helper**

In `claude_install.rs`, append next to `which_teramind_hook`:

```rust
fn which_teramind_mcp() -> anyhow::Result<PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join(if cfg!(windows) { "teramind-mcp.exe" } else { "teramind-mcp" });
            if cand.exists() { return Ok(cand); }
        }
    }
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("teramind-mcp").output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                return Ok(PathBuf::from(line.trim()));
            }
        }
    }
    anyhow::bail!("teramind-mcp binary not found next to teramind or on PATH")
}
```

- [ ] **Step 2: Bind the path early in `run()`**

In the `run()` function, just after `let teramind_hook_bin = which_teramind_hook()?;`, add:

```rust
let teramind_mcp_bin = which_teramind_mcp()?;
let mcp_bin_str = teramind_mcp_bin.to_string_lossy().into_owned();
```

- [ ] **Step 3: Add the new placeholder to the substitution chain**

Find the line:
```rust
let text = String::from_utf8_lossy(&bytes)
    .replace("@TERAMIND_PLUGIN_DIR@", &plugin_dir_str)
    .replace("@TERAMIND_HOOK_BIN@", &hook_bin_str);
```

Change to:
```rust
let text = String::from_utf8_lossy(&bytes)
    .replace("@TERAMIND_PLUGIN_DIR@", &plugin_dir_str)
    .replace("@TERAMIND_HOOK_BIN@", &hook_bin_str)
    .replace("@TERAMIND_MCP_BIN@", &mcp_bin_str);
```

- [ ] **Step 4: Smoke-test**

```bash
cargo build --workspace
TMP_CH=$(mktemp -d)
TERAMIND_PLUGIN_TEMPLATE_DIR=$(pwd)/plugins/claude \
CLAUDE_HOME=$TMP_CH ./target/debug/teramind claude install
cat "$TMP_CH/plugins/teramind/.mcp.json"
```
Expected: `.mcp.json` contains an absolute path to `teramind-mcp` (no `@TERAMIND_MCP_BIN@` placeholder remaining).

- [ ] **Step 5: Update the install integration test** at `crates/teramind/tests/claude_install.rs` to also assert the new placeholder gets substituted in `.mcp.json`:

```rust
let mcp_config = claude_home.path().join("plugins/teramind/.mcp.json");
assert!(mcp_config.exists(), ".mcp.json not present after install");
let body = std::fs::read_to_string(&mcp_config).unwrap();
assert!(!body.contains("@TERAMIND_MCP_BIN@"), "MCP placeholder left unpatched");
```

Run: `cargo test -p teramind-cli --test claude_install`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind/src/commands/claude_install.rs crates/teramind/tests/claude_install.rs
git commit -m "feat(cli): claude install substitutes @TERAMIND_MCP_BIN@ in .mcp.json"
```

---

## Section 12 — `teramind-hook` SessionStart auto-recall

### Task 15: `auto_recall::run` prints daemon-supplied markdown to stdout

**Files:**
- Create: `crates/teramind-hook/src/auto_recall.rs`
- Modify: `crates/teramind-hook/src/lib.rs`
- Modify: `crates/teramind-hook/src/main.rs`

When a SessionStart hook fires, after the `IngestEvent::SessionStart` is sent, the hook also calls `Request::AutoRecall { cwd, limit: 5 }` against the daemon. The daemon returns a markdown digest. The hook prints that digest to **stdout**, which Claude Code injects into the model's context.

- [ ] **Step 1: Add `pub mod auto_recall;` to `crates/teramind-hook/src/lib.rs`.**

- [ ] **Step 2: Write `crates/teramind-hook/src/auto_recall.rs`**

```rust
use std::path::Path;
use std::time::Duration;
use teramind_ipc::{client::{IpcClient, StreamClient}, proto::{Request, Response}, transport::connect};

/// Ask the daemon for an auto-recall digest. Prints the markdown to stdout if any.
/// Best-effort: any error silently no-ops. Never blocks Claude longer than `deadline`.
pub async fn run(socket: &Path, cwd: String, deadline: Duration) -> std::io::Result<()> {
    let result = tokio::time::timeout(deadline, async {
        let stream = connect(socket).await?;
        let mut client = StreamClient::new(stream);
        let resp = client.request(Request::AutoRecall(teramind_core::types::AutoRecallRequest { cwd, limit: 5 })).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        Ok::<_, std::io::Error>(resp)
    }).await;
    match result {
        Ok(Ok(Response::AutoRecallDigest { markdown, .. })) if !markdown.is_empty() => {
            // Claude Code reads SessionStart hook stdout as context to inject.
            println!("{markdown}");
        }
        _ => {} // silent no-op on any failure
    }
    Ok(())
}
```

- [ ] **Step 3: Modify `main.rs` to call auto_recall after a SessionStart event**

Edit `crates/teramind-hook/src/main.rs`. After the section that sends the notify, but only for `SessionStart` events. The cleanest way: keep track of the parsed input variant before the notify.

Find the block (around the notify call):

```rust
let mut client = StreamClient::new(stream);
let _ = client.notify(Notify::Ingest(envelope.clone())).await;
std::process::exit(0);
```

Replace it with:

```rust
let mut client = StreamClient::new(stream);
let is_session_start = matches!(envelope.event, teramind_core::types::ingest_event::IngestEvent::SessionStart { .. });
let session_cwd = match &envelope.event {
    teramind_core::types::ingest_event::IngestEvent::SessionStart { cwd, .. } => Some(cwd.clone()),
    _ => None,
};
let _ = client.notify(Notify::Ingest(envelope.clone())).await;
drop(client);

if is_session_start {
    if let Some(cwd) = session_cwd {
        // Best-effort auto-recall — cap at 2s so we don't block Claude.
        let _ = teramind_hook::auto_recall::run(&socket, cwd, std::time::Duration::from_secs(2)).await;
    }
}

std::process::exit(0);
```

- [ ] **Step 4: Build and smoke**

Build: `cargo build -p teramind-hook` — clean.

Manual smoke (optional): pipe a SessionStart payload through the hook against a running daemon with prior history and confirm the digest prints to stdout.

- [ ] **Step 5: Integration test for auto_recall**

Append to `crates/teramind-hook/tests/happy_path.rs`:

```rust
#[tokio::test]
async fn hook_session_start_emits_auto_recall_digest() {
    use std::io::Write;
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    // Seed prior context: one session in the same cwd.
    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let prior_sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/work-cwd", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let prior_turn = trace.upsert_turn(prior_sid, 0, now, Some("yesterday's bug fix")).await.unwrap();
    trace.finalize_turn(prior_turn, now, Some("fixed by adjusting timeout"), None, None, None, None).await.unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await.unwrap();

    // Bring up daemon with SearchRepo wired.
    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let ingest = Arc::new(IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(), sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()), session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()), diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(), dead_letter_dir: tmp.path().join("dl"),
    }));
    let handler = Arc::new(DaemonIpcHandler {
        ingest: ingest.clone(), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        search_repo: teramind_db::repos::SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    // Fire a SessionStart for a NEW session in the same cwd.
    let hook = cargo_bin("teramind-hook");
    let payload = r#"{"hook_event_name":"SessionStart","session_id":"new-sess","cwd":"/work-cwd","source":"startup"}"#;
    let mut child = Command::new(&hook)
        .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
        .env("TERAMIND_HOOK_NO_SPAWN", "1")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("Recent Teramind context") || stdout.contains("yesterday"),
            "expected auto-recall digest on stdout; got: {stdout}");

    sup.shutdown().await.unwrap();
}
```

Run: `cargo test -p teramind-hook --test happy_path hook_session_start_emits_auto_recall_digest -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-hook/src/auto_recall.rs crates/teramind-hook/src/lib.rs \
        crates/teramind-hook/src/main.rs crates/teramind-hook/tests/happy_path.rs
git commit -m "feat(hook): SessionStart auto-recall — print daemon digest to stdout for context injection"
```

---

## Section 13 — End-to-end CLI and MCP integration tests

### Task 16: CLI search end-to-end

**Files:**
- Modify: `crates/teramind/tests/smoke_e2e.rs` OR new test file

Add a new integration test that runs the daemon, seeds a turn, runs `teramind search`, and asserts the hit appears in stdout.

- [ ] **Step 1: Create `crates/teramind/tests/search_cli.rs`**

```rust
#![cfg(unix)]
use std::process::{Command, Stdio};
use std::io::Write;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}")).map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn teramind_search_returns_seeded_hit() {
    let _ = Command::new("cargo").args(["build", "--workspace"]).status();

    let tmp = tempdir().unwrap();
    let target_dir = cargo_bin("teramind").parent().unwrap().to_path_buf();
    let path_with_target = format!("{}:{}", target_dir.display(), std::env::var("PATH").unwrap_or_default());

    let env = vec![
        ("HOME", tmp.path().to_string_lossy().into_owned()),
        ("XDG_DATA_HOME", tmp.path().join("xdg-data").to_string_lossy().into_owned()),
        ("XDG_CONFIG_HOME", tmp.path().join("xdg-config").to_string_lossy().into_owned()),
        ("TERAMIND_SOCKET", tmp.path().join("t.sock").to_string_lossy().into_owned()),
        ("TERAMIND_LOG", "warn".to_string()),
        ("PATH", path_with_target),
    ];

    let teramind = cargo_bin("teramind");
    let hook = cargo_bin("teramind-hook");

    // init + start
    assert!(Command::new(&teramind).arg("init").envs(env.iter().cloned()).status().unwrap().success());
    Command::new(&teramind).arg("start").envs(env.iter().cloned()).status().unwrap();
    // Poll status up to 90s for cold-start PG.
    let mut ready = false;
    for _ in 0..90 {
        let out = Command::new(&teramind).arg("status").envs(env.iter().cloned()).output().unwrap();
        if String::from_utf8_lossy(&out.stdout).contains("uptime") {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert!(ready, "daemon never became responsive");

    // Seed a SessionStart + UserPrompt via the hook.
    for payload in &[
        r#"{"hook_event_name":"SessionStart","session_id":"cli-test","cwd":"/tmp/cli-test","source":"startup"}"#,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"cli-test","cwd":"/tmp/cli-test","prompt":"rust async deadlock"}"#,
    ] {
        let mut child = Command::new(&hook).envs(env.iter().cloned())
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap();
        child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());
    }
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Refresh MV via psql-equivalent — we don't have psql; rely on the daemon's 30s refresh.
    // Instead, wait up to 35s for the MV to refresh.
    let mut found = false;
    for _ in 0..40 {
        let out = Command::new(&teramind).args(["search", "deadlock"]).envs(env.iter().cloned()).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("deadlock") {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Cleanup daemon.
    if let Ok(pid_str) = std::fs::read_to_string(tmp.path().join("xdg-data/teramind/teramindd.pid")) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe { libc::kill(pid, libc::SIGTERM); }
        }
    }
    assert!(found, "teramind search 'deadlock' did not find the seeded prompt");
}
```

Add `libc` to dev-deps if not already (Plan A added it for smoke_e2e).

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-cli --test search_cli teramind_search_returns_seeded_hit -- --nocapture`
Expected: PASS. Slow (~60-90s on cold-start PG).

If it flakes on the MV-refresh timing, increase the 35s wait loop or trigger the refresh by querying `Request::Search` rapidly (the daemon could refresh on demand — but Plan A's 30s scheduler doesn't expose a manual trigger; live with the wait for now).

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/tests/search_cli.rs
git commit -m "test(cli): teramind search CLI end-to-end via daemon"
```

---

### Task 17: MCP server integration test

**Files:**
- Create: `crates/teramind-mcp/tests/tools.rs`

Test the MCP server end-to-end by spawning `teramind-mcp` as a child process, sending a JSON-RPC initialize then a tool call over its stdin, and asserting the response.

- [ ] **Step 1: Write the test**

```rust
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}")).map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn mcp_server_responds_to_initialize() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-mcp"]).status();
    let tmp = tempdir().unwrap();
    let mcp = cargo_bin("teramind-mcp");

    let mut child = Command::new(&mcp)
        .env("TERAMIND_SOCKET", tmp.path().join("no-daemon.sock"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();

    let stdin = child.stdin.as_mut().unwrap();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    writeln!(stdin, "{init}").unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read = reader.read_line(&mut line).unwrap();
    assert!(read > 0, "expected response line");
    assert!(line.contains("\"result\""), "expected initialize result: {line}");

    let _ = child.kill();
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-mcp --test tools mcp_server_responds_to_initialize`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-mcp/tests/tools.rs
git commit -m "test(mcp): teramind-mcp responds to MCP initialize"
```

---

## Section 14 — L4 documentation

### Task 18: Manual search smoke runbook

**Files:**
- Create: `docs/runbooks/claude-search-manual-smoke.md`

- [ ] **Step 1: Write the runbook**

```markdown
# Manual smoke: Teramind search surfaces with real Claude Code

This runbook verifies Plan C's four search surfaces against a real Claude Code session.

## Prerequisites

- Plan A + B + C merged and `cargo build --release`'d.
- `teramind init && teramind start && teramind claude install` complete.
- At least one prior Claude session's worth of captured traces in Postgres.

## Procedure

### 1. CLI search

```bash
teramind search "<a topic from a prior session>"
```

Expect: ranked hits, with snippets, scores, and timestamps. The `(N hits in M ms)` line should appear on stderr.

### 2. MCP tool from inside Claude

```bash
cd /tmp/teramind-search-smoke && claude
```

Inside Claude, ask: *"Use the teramind search tool to find anything we've talked about regarding `<topic>`. List the top 3 hits."*

Expect: Claude calls `mcp__teramind__search` (visible in tool-use indicator), receives structured Hit JSON, summarizes the hits.

### 3. Slash command

Inside Claude:

```
/teramind:search <topic>
```

Expect: equivalent to (2) but triggered by user explicitly.

### 4. Auto-recall on SessionStart

Open Claude in a directory where prior traces exist:

```bash
cd /path/with/history && claude
```

Expect: Claude's first response acknowledges or references the auto-injected "Recent Teramind context" digest from prior sessions.

### 5. Grep fallback

Stop Postgres only (kill the embedded PG child but leave teramindd running) and rerun (1):

```bash
# In a separate shell, find the PG pid and kill it
pkill -f postgres
teramind search "anything"
```

Expect: the `(degraded: Postgres unreachable …)` banner on stderr, plus best-effort hits from JSONL.

Then `teramind restart` to recover.

## Failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| CLI search returns 0 hits but you know prior traces exist | The 30s MV refresh hasn't fired yet | Wait 30 s; the daemon's scheduler will refresh `traces_fts`. Verify with `teramind status`. |
| MCP tool not visible to Claude | `.mcp.json` not patched at install time, or `teramind-mcp` not on PATH | Reinstall plugin with `teramind claude install`. |
| Slash command not visible | Plugin's `commands/` dir missing from `~/.claude/plugins/teramind/` | Reinstall plugin. |
| Auto-recall digest missing | Hook timed out or daemon AutoRecall failed | Check `~/.local/share/teramind/logs/`. Increase the 2s budget in `auto_recall.rs` if needed. |

## When to re-run this runbook

- Every change to `crates/teramindd/src/services/search.rs` or `grep_fallback.rs`.
- Every change to `teramind-mcp` tool definitions.
- Every Claude Code minor version (MCP and hook payload formats have evolved historically).
```

- [ ] **Step 2: Commit**

```bash
mkdir -p docs/runbooks
git add docs/runbooks/claude-search-manual-smoke.md
git commit -m "docs: manual smoke runbook for Teramind search surfaces"
```

---

## Plan C completion checklist

By the end of Task 18:

- `crates/teramind-mcp/` is a new workspace member binary using `rmcp` for the stdio MCP server. Three tools: `search`, `recall`, `save_skill`.
- `teramind-core::types::search` introduces `SearchRequest`, `RecallRequest`, `AutoRecallRequest`, `SaveSkillRequest`, `SearchResults`, `SkillRef`.
- `teramind-ipc::proto::Request` and `Response` extended with the 4 new variants.
- `teramind-db::repos::search::SearchRepo` exposes `fts_turns`, `trgm_diffs`, `trgm_skills`, `recent_turns_in_project`, `upsert_skill`.
- `teramindd::services::search` has `do_search`, `do_recall`, `do_auto_recall`, `do_search_with_fallback`, plus the pure ranking blend.
- `teramindd::services::grep_fallback::run` shells out to `grep` over JSONL when PG is unreachable.
- `DaemonIpcHandler` routes the new IPC variants.
- `teramind search` CLI subcommand prints hits (pretty or JSON).
- `plugins/claude/commands/{search,recall}.md` slash commands.
- `plugins/claude/.mcp.json` registers `teramind-mcp`.
- `teramind claude install` substitutes `@TERAMIND_MCP_BIN@`.
- `teramind-hook` SessionStart calls `Request::AutoRecall` and prints the digest to stdout for context injection.
- 9+ new tests across crates (unit + integration + CLI + MCP).

What Plan C does **not** ship (deferred):
- FS watcher + per-turn diff capture (Plan D — produces the `file_diffs` rows search can find).
- Installer scripts and release packaging (Plan E).
- L5 search-effectiveness benchmark with CI gates (Plan F).
- Embeddings / `pgvector` semantic search (post-spec).

---

## Plan self-review

**Spec coverage** (against `docs/superpowers/specs/2026-05-13-teramind-core-design.md` Section 6):

- §6.0 four surfaces (CLI, MCP tools, slash commands, auto-recall) — all four wired in Tasks 10, 11, 12, 15.
- §6.1 ranking blend (`0.6 fts + 0.4 trgm + 0.2 recency + 0.3 project_boost`) — implemented in `search::final_score` (Task 5).
- §6.2 Hit result type — pre-existing; populated by `rank_and_hydrate`.
- §6.3 MCP tool contract — `search(query, limit)`, `recall(cwd, file_paths, symbols, stack_traces, limit)`, `save_skill(name, description, body)` — Tasks 2 (types) + 11 (rmcp wiring).
- §6.4 auto-recall (three parallel queries: recent turns, similar diffs, top skills, merged into 4KB markdown) — Task 6's `do_auto_recall` does the recent-turns half; the other two are explicit follow-ons noted in the runbook (worth a small follow-up to enrich the digest).
- §6.5 grep fallback (kill-PG, regex JSONL) — Tasks 7 + 8.

**Placeholder scan:** none. Each step has full code; each command has expected output.

**Type consistency:**
- `SearchRepo::fts_turns(&str, u32)` → `Vec<RankedTurn>` — same shape consumed by `do_search` in Task 6.
- `Hit` variants (Turn, ToolCall, FileDiff, Skill) — same in `rank_and_hydrate` (Task 5), `grep_fallback::run` (Task 7), CLI presenter (Task 10), MCP tool result body (Task 11).
- `DaemonIpcHandler` field additions (`search_repo`, `jsonl_dir`) — propagated to every test that constructs `DaemonIpcHandler` (Task 9 Step 4 lists each).
- `auto_recall::run(socket, cwd, deadline)` — single call site in `main.rs` (Task 15 Step 3).

**Open follow-ups deferred to a future Plan C.1:**
- `do_auto_recall` only renders recent turns; spec §6.4 wants 3-query parallel merge with diffs and skills. Adds richness; not blocking.
- The `_grep` CLI flag is parsed but unused. Plumb through to force grep-mode in `do_search_with_fallback` when set. Trivial.
- CI integration test for `search_cli` is slow (~90s). Move it to nightly tag in `.github/workflows/ci.yml` once `[full]`-label routing is in place.

---

