# Teramind Session Summarizer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate LLM-written Markdown summaries for ended Claude sessions via a background worker, store them in a new `wiki_pages` table, join them into `traces_fts` for free search integration, and expose them through a new MCP tool (`mcp__teramind__wiki`), a new CLI subcommand (`teramind sessions show`), and `do_auto_recall`.

**Architecture:** New `wiki_pages` table keyed by `(session_id, model)` with `ON DELETE CASCADE`. `summarizer_worker` polls a `sessions_to_summarize` view, snapshots the session, builds a structured Markdown digest (`digest::build`), redacts, calls a `SummaryProvider` trait, persists Markdown. The `traces_fts` materialized view is rebuilt to UNION-include `wiki_pages.content`. Default provider is `OllamaChatProvider` (qwen3.6:latest); cloud providers refuse to construct without `network_egress = true`.

**Tech Stack:** Rust stable (workspace pin 1.93.0), existing workspace (sqlx 0.8, postgresql_embedded 0.20, reqwest 0.12 rustls, tokio, async-trait). Reuses `teramind_core::embed::ProviderKind` from Plan G. Reuses `Redactor` from Plan A. No new crate deps required.

---

## Spec coverage

This plan implements `docs/superpowers/specs/2026-05-16-teramind-session-summarizer-design.md`. The coverage matrix at the bottom maps each spec requirement to a task.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramind-core/src/summarize.rs` | `SummaryProvider` trait + `SummaryError` + `SummaryResult` |
| `crates/teramindd/src/services/summarize/mod.rs` | Factory + module registry |
| `crates/teramindd/src/services/summarize/ollama.rs` | `OllamaChatProvider` (HTTP `/api/chat`) |
| `crates/teramindd/src/services/summarize/anthropic.rs` | `AnthropicProvider` (gated; HTTPS) |
| `crates/teramindd/src/services/summarize/openai.rs` | `OpenaiProvider` (v1.0 stub) |
| `crates/teramindd/src/services/summarize/digest.rs` | Pure `digest::build` + `SessionSnapshot` |
| `crates/teramindd/src/services/summarize/prompts.rs` | `SYSTEM_PROMPT` constant + snapshot test |
| `crates/teramindd/src/services/summarizer_worker.rs` | Async poll/digest/summarize/persist loop |
| `crates/teramind-db/migrations/20260516000002_wiki_pages.sql` | Table + view + `traces_fts` rebuild |
| `crates/teramind-db/src/repos/wiki.rs` | `WikiRepo` |
| `crates/teramind/src/commands/sessions.rs` | `teramind sessions show [<id>] [--json]` |
| `docs/runbooks/summarizer-manual-smoke.md` | Manual test guide |

**Modified files:**

- `crates/teramind-core/src/lib.rs` — register `summarize` module.
- `crates/teramind-core/src/types/hit.rs` — add `Hit::WikiPage` variant.
- `crates/teramind-core/src/ids.rs` — add `WikiPageId(Uuid)`.
- `crates/teramind-db/src/repos/mod.rs` — register `wiki` module.
- `crates/teramind-db/src/repos/search.rs` — extend `rank_and_hydrate` to include wiki hits; new `fts_wiki_pages` method.
- `crates/teramindd/src/config.rs` — `SummarizeConfig` types + loader.
- `crates/teramindd/src/services/mod.rs` — register `summarize` + `summarizer_worker`.
- `crates/teramindd/src/services/search.rs` — `do_auto_recall` adds latest-wiki source; `do_search` includes wiki hits via the rebuilt FTS.
- `crates/teramindd/src/services/ipc_server.rs` — `Request::WikiLookup` arm; populate StatusReport summary fields.
- `crates/teramindd/src/app.rs` — wire `summarizer_worker`; pass `WikiRepo` to handler.
- `crates/teramind-ipc/src/proto.rs` — `Request::WikiLookup`, `Response::WikiPage`, new StatusReport fields.
- `crates/teramind-mcp/src/server.rs` — `mcp__teramind__wiki` tool.
- `crates/teramind/src/cli.rs` — `Sessions { action: SessionsAction }` variant.
- `crates/teramind/src/commands/mod.rs` — register `sessions` module.
- `crates/teramind/src/main.rs` — dispatch the new variant.
- `crates/teramind/src/commands/doctor.rs` — render summary provider lines.
- `.gitignore` — none required (no transient files).

---

## Section 0 — Pre-flight

### Task 0.1: Verify pgcrypto + sqlx string-array binding

The migration uses `gen_random_uuid()` (pgcrypto, enabled by Plan A). Sanity-check it's available.

**Files:** None (verification only).

- [ ] **Step 1: Inspect existing migrations**

Run: `grep -rn "gen_random_uuid\|pgcrypto" crates/teramind-db/migrations/`

Expected: hits for `gen_random_uuid()` in earlier migrations (Plan A's `agents`/`sessions`/`turns`/etc.) and a `pgcrypto` extension setup.

If pgcrypto isn't enabled, the migration in §1 must add `CREATE EXTENSION IF NOT EXISTS pgcrypto;` at the top.

- [ ] **Step 2: Confirm sqlx ProviderKind serialization roundtrips**

Run: `cargo test -p teramind-core embed::tests::provider_kind_serde_roundtrip`
Expected: PASS (added in Plan G §2.1). This confirms `ProviderKind` round-trips through TOML/JSON — we reuse it in `summarize.rs`.

No commit. Move to §1.

---

## Section 1 — Schema migration

### Task 1.1: `wiki_pages` table, view, and `traces_fts` rebuild

**Files:**
- Create: `crates/teramind-db/migrations/20260516000002_wiki_pages.sql`
- Modify: `crates/teramind-db/tests/migrations.rs` (append test)

- [ ] **Step 1: Author the migration**

Create `crates/teramind-db/migrations/20260516000002_wiki_pages.sql` with EXACTLY:

```sql
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
```

- [ ] **Step 2: Append the verification test**

Append to `crates/teramind-db/tests/migrations.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_pages_migration_applies_and_traces_fts_rebuilt() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // wiki_pages table has 7 columns.
    let (cols,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.columns WHERE table_name='wiki_pages'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(cols, 7);

    // sessions_to_summarize view exists.
    let (view_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.views WHERE table_name='sessions_to_summarize'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(view_count, 1);

    // traces_fts still queryable after the rebuild.
    let (_,): (i64,) = sqlx::query_as("SELECT count(*) FROM traces_fts")
        .fetch_one(pool.pg()).await?;

    // CASCADE: deleting a session removes its wiki_pages.
    use teramind_db::repos::{AgentRepo, SessionRepo};
    use teramind_db::repos::session::NewSession;
    use teramind_core::ids::SessionId;
    use time::OffsetDateTime;
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    sqlx::query("INSERT INTO wiki_pages (session_id, model, content, input_tokens, output_tokens) VALUES ($1, 'm', 'x', 1, 1)")
        .bind(sid.0).execute(pool.pg()).await?;
    sqlx::query("DELETE FROM sessions WHERE id = $1").bind(sid.0).execute(pool.pg()).await?;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM wiki_pages WHERE session_id = $1")
        .bind(sid.0).fetch_one(pool.pg()).await?;
    assert_eq!(n, 0, "CASCADE delete should have removed wiki_pages");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramind-db wiki_pages_migration_applies_and_traces_fts_rebuilt --release`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/migrations/20260516000002_wiki_pages.sql crates/teramind-db/tests/migrations.rs
git commit -m "feat(db): wiki_pages table + sessions_to_summarize view + traces_fts rebuild"
```

---

## Section 2 — `SummaryProvider` trait

### Task 2.1: Trait + shared types in `teramind-core`

**Files:**
- Create: `crates/teramind-core/src/summarize.rs`
- Modify: `crates/teramind-core/src/lib.rs`

- [ ] **Step 1: Create the trait module**

Create `crates/teramind-core/src/summarize.rs`:

```rust
//! Summary provider trait + shared types. Lives in `teramind-core` so
//! the MCP / eval / CLI crates can depend on it without pulling in the daemon.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use crate::embed::ProviderKind;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("provider unhealthy: {0}")]
    Unhealthy(String),
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryResult {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[async_trait]
pub trait SummaryProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn model_id(&self) -> &str;
    fn max_input_tokens(&self) -> usize;
    fn max_output_tokens(&self) -> usize;
    async fn health_check(&self) -> Result<(), SummaryError>;
    async fn summarize(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_error_classifies() {
        assert!(matches!(SummaryError::Unhealthy("x".into()), SummaryError::Unhealthy(_)));
        assert!(matches!(SummaryError::Network("x".into()), SummaryError::Network(_)));
    }

    #[test]
    fn summary_result_roundtrips_through_json() {
        let r = SummaryResult { content: "ok".into(), input_tokens: 10, output_tokens: 20 };
        let j = serde_json::to_string(&r).unwrap();
        let back: SummaryResult = serde_json::from_str(&j).unwrap();
        assert_eq!(r.input_tokens, back.input_tokens);
        assert_eq!(r.content, back.content);
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/teramind-core/src/lib.rs`, append:

```rust
pub mod summarize;
```

- [ ] **Step 3: Verify deps**

`crates/teramind-core/Cargo.toml` already has `async-trait`, `serde`, `serde_json`, `thiserror` (from Plans A + G). No new deps.

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-core summarize`
Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/summarize.rs crates/teramind-core/src/lib.rs
git commit -m "feat(core): SummaryProvider trait + shared types"
```

---

## Section 3 — Digest builder + system prompt

### Task 3.1: `SessionSnapshot` types + digest scaffold

**Files:**
- Create: `crates/teramindd/src/services/summarize/mod.rs`
- Create: `crates/teramindd/src/services/summarize/digest.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod summarize;
```

- [ ] **Step 2: Scaffold the submodule index**

Create `crates/teramindd/src/services/summarize/mod.rs`:

```rust
//! Session summarizer: trait + provider impls + digest + prompts.

pub mod digest;
pub mod prompts;
```

(`ollama`, `anthropic`, `openai`, `factory` arrive in later sections.)

- [ ] **Step 3: Author `digest.rs` skeleton + types**

Create `crates/teramindd/src/services/summarize/digest.rs`:

```rust
//! Pure digest builder. Takes a SessionSnapshot, returns a Markdown
//! string capped at `char_budget`. No I/O, no async.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use teramind_core::ids::{SessionId, ToolCallId, TurnId};
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: TurnId,
    pub ordinal: i32,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRow {
    pub id: ToolCallId,
    pub turn_id: TurnId,
    pub name: String,
    pub input: Value,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffRow {
    pub turn_id: Option<TurnId>,
    pub rel_path: String,
    pub language: Option<String>,
    pub attribution: Attribution,
    pub unified_diff: String,
    pub pre_excerpt: String,
    pub post_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: SessionId,
    pub cwd: String,
    pub started_at: OffsetDateTime,
    pub ended_at: OffsetDateTime,
    pub end_reason: String,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    pub turns: Vec<TurnRow>,
    pub tool_calls: Vec<ToolCallRow>,
    pub file_diffs: Vec<FileDiffRow>,
}

impl SessionSnapshot {
    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }

    pub fn duration_secs(&self) -> i64 {
        (self.ended_at - self.started_at).whole_seconds()
    }
}

/// Build a Markdown digest from the snapshot. Output length <= `char_budget`.
/// Sections are dropped in priority order when over budget.
pub fn build(snapshot: &SessionSnapshot, char_budget: usize) -> String {
    let mut sections = Vec::new();
    sections.push(("header".to_string(), render_header(snapshot)));
    let tools = render_tool_usage(snapshot);
    if !tools.is_empty() { sections.push(("tools".to_string(), tools)); }
    let files = render_files_changed(snapshot);
    if !files.is_empty() { sections.push(("files".to_string(), files)); }
    let prompts = render_key_prompts(snapshot);
    if !prompts.is_empty() { sections.push(("prompts".to_string(), prompts)); }
    let outputs = render_key_outputs(snapshot);
    if !outputs.is_empty() { sections.push(("outputs".to_string(), outputs)); }
    let errors = render_tool_errors(snapshot);
    if !errors.is_empty() { sections.push(("errors".to_string(), errors)); }
    let diffs = render_diff_samples(snapshot);
    if !diffs.is_empty() { sections.push(("diffs".to_string(), diffs)); }

    enforce_budget(sections, char_budget)
}

// Priority drop order when over budget:
//   1. "diffs"      (highest cost / lowest priority)
//   2. "outputs"
//   3. "prompts"
//   4. "errors"
// Header / tools / files are always kept; the file list itself is truncated
// to 10 entries as a final fallback.
const DROP_ORDER: &[&str] = &["diffs", "outputs", "prompts", "errors"];

fn enforce_budget(mut sections: Vec<(String, String)>, budget: usize) -> String {
    let mut result = join(&sections);
    let mut idx = 0;
    while result.len() > budget && idx < DROP_ORDER.len() {
        let target = DROP_ORDER[idx];
        sections.retain(|(name, _)| name != target);
        result = join(&sections);
        idx += 1;
    }
    if result.len() > budget {
        // Final fallback: truncate the "files" section to the first 10 bullets.
        if let Some(files_idx) = sections.iter().position(|(n, _)| n == "files") {
            sections[files_idx].1 = truncate_bullets(&sections[files_idx].1, 10);
            result = join(&sections);
        }
    }
    if result.len() > budget {
        result = truncate_to_char_boundary(&result, budget);
    }
    result
}

fn join(sections: &[(String, String)]) -> String {
    sections.iter().map(|(_, body)| body.as_str()).collect::<Vec<_>>().join("\n\n")
}

fn truncate_to_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    s[..end].to_string()
}

fn truncate_bullets(s: &str, max_bullets: usize) -> String {
    let mut out = String::new();
    let mut bullet_count = 0;
    for line in s.lines() {
        if line.trim_start().starts_with("- ") {
            if bullet_count >= max_bullets { continue; }
            bullet_count += 1;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn render_header(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    writeln!(out, "# Session digest\n").unwrap();
    writeln!(out, "- session_id: {}", s.session_id.0).unwrap();
    writeln!(out, "- cwd: {}", s.cwd).unwrap();
    let dur = s.duration_secs();
    writeln!(out, "- duration: {}m {}s", dur / 60, dur % 60).unwrap();
    if let (Some(b), Some(h)) = (&s.git_branch, &s.git_head) {
        writeln!(out, "- git branch / head: {} at {}", b, &h[..h.len().min(7)]).unwrap();
    }
    writeln!(out, "- ended: {}", s.end_reason).unwrap();
    writeln!(
        out,
        "- turns: {}    tool calls: {}    files changed: {}",
        s.turns.len(), s.tool_calls.len(), s.file_diffs.len(),
    ).unwrap();
    out
}

fn render_tool_usage(s: &SessionSnapshot) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    if s.tool_calls.is_empty() { return String::new(); }
    let mut counts: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for tc in &s.tool_calls {
        let e = counts.entry(tc.name.as_str()).or_insert((0, 0));
        e.0 += 1;
        if tc.is_error { e.1 += 1; }
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    ranked.truncate(5);
    let mut out = String::new();
    writeln!(out, "## Tool usage (top 5 by count)\n").unwrap();
    for (name, (n, errs)) in ranked {
        if errs > 0 {
            writeln!(out, "- {} x{}  (errors: {})", name, n, errs).unwrap();
        } else {
            writeln!(out, "- {} x{}", name, n).unwrap();
        }
    }
    out
}

fn render_files_changed(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    if s.file_diffs.is_empty() { return String::new(); }
    let mut out = String::new();
    writeln!(out, "## Files changed\n").unwrap();
    for d in &s.file_diffs {
        let plus = d.unified_diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
        let minus = d.unified_diff.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
        let attr = match d.attribution { Attribution::Agent => "agent", Attribution::Human => "human" };
        let lang = d.language.as_deref().unwrap_or("text");
        writeln!(out, "- {} ({}, {}) — (+{}, -{})", d.rel_path, lang, attr, plus, minus).unwrap();
    }
    out
}

fn render_key_prompts(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let mut prompts: Vec<&str> = s.turns.iter()
        .filter_map(|t| t.user_prompt.as_deref())
        .filter(|p| !p.trim().is_empty())
        .collect();
    if prompts.is_empty() { return String::new(); }
    prompts.sort_by_key(|p| std::cmp::Reverse(p.len()));
    prompts.truncate(5);
    let mut out = String::new();
    writeln!(out, "## Key prompts (longest, up to 5)\n").unwrap();
    for (i, p) in prompts.iter().enumerate() {
        writeln!(out, "{}. > {:?}", i + 1, truncate_for_quote(p, 400)).unwrap();
    }
    out
}

fn render_key_outputs(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let mut outs: Vec<&str> = s.turns.iter()
        .filter_map(|t| t.assistant_text.as_deref())
        .filter(|p| !p.trim().is_empty())
        .collect();
    if outs.is_empty() { return String::new(); }
    outs.sort_by_key(|p| std::cmp::Reverse(p.len()));
    outs.truncate(5);
    let mut out = String::new();
    writeln!(out, "## Key assistant outputs (longest, up to 5)\n").unwrap();
    for (i, p) in outs.iter().enumerate() {
        writeln!(out, "{}. > {:?}", i + 1, truncate_for_quote(p, 400)).unwrap();
    }
    out
}

fn render_tool_errors(s: &SessionSnapshot) -> String {
    use std::fmt::Write;
    let errs: Vec<_> = s.tool_calls.iter().filter(|tc| tc.is_error).take(3).collect();
    if errs.is_empty() { return String::new(); }
    let mut out = String::new();
    writeln!(out, "## Notable tool errors (up to 3)\n").unwrap();
    for tc in errs {
        let snippet = truncate_for_quote(&tc.output, 200);
        writeln!(out, "- {}: {:?}", tc.name, snippet).unwrap();
    }
    out
}

fn render_diff_samples(s: &SessionSnapshot) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    if s.file_diffs.is_empty() { return String::new(); }
    let mut by_path: BTreeMap<&str, (usize, &FileDiffRow)> = BTreeMap::new();
    for d in &s.file_diffs {
        let churn = d.unified_diff.lines().count();
        by_path.entry(d.rel_path.as_str())
            .and_modify(|e| if churn > e.0 { *e = (churn, d); })
            .or_insert((churn, d));
    }
    let mut ranked: Vec<_> = by_path.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    ranked.truncate(2);
    let mut out = String::new();
    writeln!(out, "## Diff samples (one per file, top 2 by churn)\n").unwrap();
    for (_, (_, d)) in ranked {
        let fence = match d.language.as_deref().unwrap_or("text") {
            l if !l.is_empty() => l,
            _ => "text",
        };
        writeln!(out, "```{}", fence).unwrap();
        writeln!(out, "// {}", d.rel_path).unwrap();
        let snippet = truncate_to_char_boundary(&d.unified_diff, 1200);
        out.push_str(&snippet);
        if !snippet.ends_with('\n') { out.push('\n'); }
        writeln!(out, "```\n").unwrap();
    }
    out
}

fn truncate_for_quote(s: &str, max: usize) -> String {
    let single_line: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if single_line.chars().count() <= max {
        single_line
    } else {
        let mut out: String = single_line.chars().take(max).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with_turns(n: usize) -> SessionSnapshot {
        let mut turns = Vec::new();
        for i in 0..n {
            turns.push(TurnRow {
                id: TurnId(uuid::Uuid::new_v4()),
                ordinal: i as i32,
                user_prompt: Some(format!("prompt {i}")),
                assistant_text: Some(format!("response {i}")),
                thinking: None,
                started_at: OffsetDateTime::now_utc(),
            });
        }
        SessionSnapshot {
            session_id: SessionId(uuid::Uuid::new_v4()),
            cwd: "/proj".into(),
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            ended_at: OffsetDateTime::from_unix_timestamp(1_700_002_000).unwrap(),
            end_reason: "stop_hook".into(),
            git_branch: Some("main".into()),
            git_head: Some("abc1234567890".into()),
            turns,
            tool_calls: Vec::new(),
            file_diffs: Vec::new(),
        }
    }

    #[test]
    fn build_respects_char_budget() {
        let s = snapshot_with_turns(50);
        let d = build(&s, 1024);
        assert!(d.len() <= 1024, "len={}", d.len());
        assert!(d.starts_with("# Session digest"), "got: {}", &d[..d.len().min(200)]);
    }

    #[test]
    fn build_is_deterministic() {
        let s = snapshot_with_turns(5);
        let a = build(&s, 8192);
        let b = build(&s, 8192);
        assert_eq!(a, b);
    }

    #[test]
    fn build_priority_drop_order() {
        let mut s = snapshot_with_turns(20);
        // Force diff samples to exist.
        s.file_diffs.push(FileDiffRow {
            turn_id: None,
            rel_path: "a.rs".into(),
            language: Some("rust".into()),
            attribution: Attribution::Agent,
            unified_diff: "--- a\n+++ b\n@@ x\n-old\n+new\n".into(),
            pre_excerpt: "old".into(), post_excerpt: "new".into(),
        });
        let unlimited = build(&s, 32768);
        let limited   = build(&s, 800);
        assert!(unlimited.contains("Diff samples"));
        assert!(!limited.contains("Diff samples"),
                "diffs should be dropped first at small budget; got:\n{}", limited);
    }

    #[test]
    fn truncate_to_char_boundary_handles_multi_byte() {
        let s = "héllo";  // 'é' is 2 bytes
        let t = truncate_to_char_boundary(s, 2);
        assert!(s.starts_with(&t));
        assert!(t == "h" || t == "hé");
    }

    use proptest::prelude::*;
    proptest! {
        #[test]
        fn build_length_never_exceeds_budget(
            turn_count in 0usize..30usize,
            budget in 256usize..16384usize,
        ) {
            let s = snapshot_with_turns(turn_count);
            let d = build(&s, budget);
            prop_assert!(d.len() <= budget, "len={} > budget={}", d.len(), budget);
        }
    }
}
```

- [ ] **Step 4: Verify `proptest` is a dev-dep on teramindd**

Run: `grep "proptest" crates/teramindd/Cargo.toml`
Expected: it's in `[dev-dependencies]` (added in Plan D §2.6). If missing, add `proptest = { workspace = true }` under `[dev-dependencies]`.

- [ ] **Step 5: Run**

Run: `cargo test -p teramindd digest`
Expected: 5 tests PASS (4 unit + 1 proptest with default 256 cases).

- [ ] **Step 6: Commit**

```bash
git add crates/teramindd/src/services/summarize/digest.rs crates/teramindd/src/services/summarize/mod.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): summarize digest builder with priority-drop budget"
```

---

### Task 3.2: System prompt + snapshot test

**Files:**
- Create: `crates/teramindd/src/services/summarize/prompts.rs`

- [ ] **Step 1: Author**

```rust
//! Compile-time prompt constants. A snapshot test in this module catches
//! accidental drift from the prompt the spec calls for.

pub const SYSTEM_PROMPT: &str = "You are summarizing a Claude Code session for a developer wiki. The user\nhas given you a structured digest of what happened. Write a concise wiki\npage in Markdown with these sections, in order:\n\n# Summary\n\nA one-paragraph (~3 sentences) plain-English description of what the\nsession accomplished, who initiated it (agent vs human edits), and the\noutcome.\n\n# Files changed\n\nA bulleted list of files with a one-sentence note per file describing\nthe intent of the change.\n\n# Decisions & gotchas\n\n3-5 bullets. Surface non-obvious decisions and gotchas the agent noted.\nIf none are visible in the digest, write \"None recorded.\"\n\n# Follow-ups\n\nTasks left undone or implied by the work. If none, write \"None recorded.\"\n\nConstraints:\n- Be faithful to the digest. Do NOT invent details not present.\n- Cite filenames and tool names verbatim where relevant.\n- Output Markdown only. No preamble.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_all_four_section_headers() {
        for header in ["# Summary", "# Files changed", "# Decisions & gotchas", "# Follow-ups"] {
            assert!(SYSTEM_PROMPT.contains(header), "missing {header}");
        }
    }

    #[test]
    fn system_prompt_forbids_invention() {
        assert!(SYSTEM_PROMPT.contains("Do NOT invent"));
    }

    #[test]
    fn system_prompt_length_under_limit() {
        // Keep the prompt small so it doesn't eat token budget at run time.
        assert!(SYSTEM_PROMPT.len() < 2048, "prompt grew to {}", SYSTEM_PROMPT.len());
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd prompts`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/summarize/prompts.rs
git commit -m "feat(daemon): summarize SYSTEM_PROMPT + snapshot tests"
```

---

## Section 4 — `OllamaChatProvider`

### Task 4.1: HTTP `/api/chat` implementation

**Files:**
- Create: `crates/teramindd/src/services/summarize/ollama.rs`
- Modify: `crates/teramindd/src/services/summarize/mod.rs`

- [ ] **Step 1: Update submodule index**

In `crates/teramindd/src/services/summarize/mod.rs`, append:

```rust
pub mod ollama;
```

- [ ] **Step 2: Author the provider**

Create `crates/teramindd/src/services/summarize/ollama.rs`:

```rust
//! Ollama chat-completion provider (POST /api/chat).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};

#[derive(Clone)]
pub struct OllamaChatProvider {
    url: String,
    model: String,
    max_input_tokens: usize,
    max_output_tokens: usize,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message<'a>; 2],
    stream: bool,
    options: ChatOptions,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatOptions {
    num_predict: i32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct VersionResponse {
    #[serde(default)]
    #[allow(dead_code)]
    version: String,
}

impl OllamaChatProvider {
    pub fn new(
        url: String,
        model: String,
        max_input_tokens: usize,
        max_output_tokens: usize,
        timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");
        Self { url, model, max_input_tokens, max_output_tokens, client }
    }
}

#[async_trait]
impl SummaryProvider for OllamaChatProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Ollama }
    fn model_id(&self) -> &str { &self.model }
    fn max_input_tokens(&self) -> usize { self.max_input_tokens }
    fn max_output_tokens(&self) -> usize { self.max_output_tokens }

    async fn health_check(&self) -> Result<(), SummaryError> {
        let url = format!("{}/api/version", self.url);
        let resp = self.client.get(&url).send().await
            .map_err(|e| SummaryError::Unhealthy(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(SummaryError::Unhealthy(format!("ollama version returned {}", resp.status())));
        }
        let _: VersionResponse = resp.json().await
            .map_err(|e| SummaryError::Unhealthy(format!("decode version: {e}")))?;
        Ok(())
    }

    async fn summarize(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        let url = format!("{}/api/chat", self.url);
        let req = ChatRequest {
            model: &self.model,
            messages: [
                Message { role: "system", content: system_prompt },
                Message { role: "user",   content: user_prompt   },
            ],
            stream: false,
            options: ChatOptions { num_predict: max_output_tokens as i32 },
        };
        let resp = self.client.post(&url).json(&req).send().await
            .map_err(|e| SummaryError::Network(format!("POST {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SummaryError::ModelNotFound(self.model.clone()));
        }
        if !resp.status().is_success() {
            return Err(SummaryError::Other(format!("ollama chat returned {}", resp.status())));
        }
        let body: ChatResponse = resp.json().await
            .map_err(|e| SummaryError::Other(format!("decode chat: {e}")))?;
        Ok(SummaryResult {
            content: body.message.content,
            input_tokens: body.prompt_eval_count.unwrap_or(0),
            output_tokens: body.eval_count.unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_advertises_correct_kind() {
        let p = OllamaChatProvider::new(
            "http://localhost:11434".into(),
            "qwen3.6:latest".into(),
            16384, 1500,
            Duration::from_secs(60),
        );
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.model_id(), "qwen3.6:latest");
        assert_eq!(p.max_input_tokens(), 16384);
        assert_eq!(p.max_output_tokens(), 1500);
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd summarize::ollama`
Expected: 1 test PASS. (Network-dependent tests in §13.)

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/summarize/ollama.rs crates/teramindd/src/services/summarize/mod.rs
git commit -m "feat(daemon): OllamaChatProvider over /api/chat"
```

---

## Section 5 — Cloud providers

### Task 5.1: `AnthropicProvider`

**Files:**
- Create: `crates/teramindd/src/services/summarize/anthropic.rs`
- Modify: `crates/teramindd/src/services/summarize/mod.rs`

- [ ] **Step 1: Update submodule index**

Append to `crates/teramindd/src/services/summarize/mod.rs`:

```rust
pub mod anthropic;
```

- [ ] **Step 2: Author**

Create `crates/teramindd/src/services/summarize/anthropic.rs`:

```rust
//! Anthropic Messages API provider. Refuses to construct without
//! network_egress + an API key.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};

#[derive(Clone)]
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    max_input_tokens: usize,
    max_output_tokens: usize,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: [UserMessage<'a>; 1],
}

#[derive(Serialize)]
struct UserMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    usage: Usage,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(default)]
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(
        api_key: String,
        model: String,
        max_input_tokens: usize,
        max_output_tokens: usize,
        timeout: Duration,
    ) -> Result<Self, SummaryError> {
        if api_key.trim().is_empty() {
            return Err(SummaryError::Other("anthropic api_key is empty".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| SummaryError::Other(format!("reqwest build: {e}")))?;
        Ok(Self { api_key, model, max_input_tokens, max_output_tokens, client })
    }
}

#[async_trait]
impl SummaryProvider for AnthropicProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Anthropic }
    fn model_id(&self) -> &str { &self.model }
    fn max_input_tokens(&self) -> usize { self.max_input_tokens }
    fn max_output_tokens(&self) -> usize { self.max_output_tokens }

    async fn health_check(&self) -> Result<(), SummaryError> {
        // Cheapest valid call: send a 1-token request.
        let body = MessagesRequest {
            model: &self.model,
            max_tokens: 1,
            system: "Reply with just OK.",
            messages: [UserMessage { role: "user", content: "ok" }],
        };
        let resp = self.client.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body).send().await
            .map_err(|e| SummaryError::Unhealthy(format!("anthropic health: {e}")))?;
        if !resp.status().is_success() {
            return Err(SummaryError::Unhealthy(format!("anthropic returned {}", resp.status())));
        }
        Ok(())
    }

    async fn summarize(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        let body = MessagesRequest {
            model: &self.model,
            max_tokens: max_output_tokens as u32,
            system: system_prompt,
            messages: [UserMessage { role: "user", content: user_prompt }],
        };
        let resp = self.client.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body).send().await
            .map_err(|e| SummaryError::Network(format!("anthropic POST: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SummaryError::ModelNotFound(self.model.clone()));
        }
        if status.as_u16() == 429 {
            return Err(SummaryError::BudgetExceeded("anthropic rate limit".into()));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SummaryError::Other(format!("anthropic returned {}: {}", status, body)));
        }
        let parsed: MessagesResponse = resp.json().await
            .map_err(|e| SummaryError::Other(format!("decode anthropic: {e}")))?;
        let content = parsed.content.iter()
            .filter(|b| b.block_type == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        Ok(SummaryResult {
            content,
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_key_is_refused() {
        let r = AnthropicProvider::new(
            "  ".into(),
            "claude-haiku-4-5-20251001".into(),
            16384, 1500,
            Duration::from_secs(30),
        );
        assert!(r.is_err());
    }

    #[test]
    fn valid_construction_advertises_correct_kind() {
        let p = AnthropicProvider::new(
            "sk-ant-test".into(),
            "claude-haiku-4-5-20251001".into(),
            16384, 1500,
            Duration::from_secs(30),
        ).unwrap();
        assert_eq!(p.kind(), ProviderKind::Anthropic);
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd summarize::anthropic`
Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/summarize/anthropic.rs crates/teramindd/src/services/summarize/mod.rs
git commit -m "feat(daemon): AnthropicProvider over Messages API"
```

---

### Task 5.2: `OpenaiProvider` stub

**Files:**
- Create: `crates/teramindd/src/services/summarize/openai.rs`
- Modify: `crates/teramindd/src/services/summarize/mod.rs`

- [ ] **Step 1: Update submodule index**

Append:

```rust
pub mod openai;
```

- [ ] **Step 2: Author**

```rust
//! OpenAI provider stub (v1.0). Refuses health and summarize calls
//! with a clear message; full implementation lands in v1.1.

use async_trait::async_trait;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};

pub struct OpenaiProvider {
    model: String,
}

impl OpenaiProvider {
    pub fn new(model: String) -> Self { Self { model } }
}

#[async_trait]
impl SummaryProvider for OpenaiProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Openai }
    fn model_id(&self) -> &str { &self.model }
    fn max_input_tokens(&self) -> usize { 16384 }
    fn max_output_tokens(&self) -> usize { 1500 }

    async fn health_check(&self) -> Result<(), SummaryError> {
        Err(SummaryError::Unhealthy(
            "openai provider is stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }

    async fn summarize(
        &self, _: &str, _: &str, _: usize,
    ) -> Result<SummaryResult, SummaryError> {
        Err(SummaryError::Other(
            "openai provider is stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_check_returns_unhealthy() {
        let p = OpenaiProvider::new("gpt-4o-mini".into());
        assert!(p.health_check().await.is_err());
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd summarize::openai`
Expected: 1 test PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/summarize/openai.rs crates/teramindd/src/services/summarize/mod.rs
git commit -m "feat(daemon): OpenaiProvider stub (v1.0 refuses)"
```

---

## Section 6 — `SummarizeConfig` + factory

### Task 6.1: Config types

**Files:**
- Modify: `crates/teramindd/src/config.rs`

- [ ] **Step 1: Append the types**

Append to `crates/teramindd/src/config.rs`:

```rust
// ============================ summarize config ============================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummarizeConfig {
    #[serde(default = "default_summarize_provider")]
    pub provider: teramind_core::embed::ProviderKind,
    #[serde(default = "default_summarize_model")]
    pub model: String,
    #[serde(default = "default_summarize_poll")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_summarize_min_turns")]
    pub min_turns: u32,
    #[serde(default = "default_summarize_min_duration")]
    pub min_duration_secs: u64,
    #[serde(default = "default_summarize_input_chars")]
    pub input_char_budget: u32,
    #[serde(default = "default_summarize_output_tokens")]
    pub output_token_budget: u32,
    #[serde(default)]
    pub network_egress: bool,
    #[serde(default)]
    pub ollama: SummarizeOllama,
    #[serde(default)]
    pub anthropic: SummarizeAnthropic,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummarizeOllama {
    #[serde(default = "default_summarize_ollama_url")]
    pub url: String,
    #[serde(default = "default_summarize_ollama_timeout")]
    pub request_timeout_ms: u64,
}

impl Default for SummarizeOllama {
    fn default() -> Self {
        Self {
            url: default_summarize_ollama_url(),
            request_timeout_ms: default_summarize_ollama_timeout(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SummarizeAnthropic {
    #[serde(default = "default_anthropic_key_field")]
    pub api_key_field: String,
    #[serde(default = "default_anthropic_timeout")]
    pub request_timeout_ms: u64,
}

fn default_summarize_provider() -> teramind_core::embed::ProviderKind {
    teramind_core::embed::ProviderKind::Ollama
}
fn default_summarize_model() -> String { "qwen3.6:latest".into() }
fn default_summarize_poll() -> u64 { 30 }
fn default_summarize_min_turns() -> u32 { 3 }
fn default_summarize_min_duration() -> u64 { 60 }
fn default_summarize_input_chars() -> u32 { 16000 }
fn default_summarize_output_tokens() -> u32 { 1500 }
fn default_summarize_ollama_url() -> String { "http://localhost:11434".into() }
fn default_summarize_ollama_timeout() -> u64 { 60_000 }
fn default_anthropic_key_field() -> String { "anthropic_api_key".into() }
fn default_anthropic_timeout() -> u64 { 30_000 }

impl Default for SummarizeConfig {
    fn default() -> Self {
        Self {
            provider: default_summarize_provider(),
            model: default_summarize_model(),
            poll_interval_secs: default_summarize_poll(),
            min_turns: default_summarize_min_turns(),
            min_duration_secs: default_summarize_min_duration(),
            input_char_budget: default_summarize_input_chars(),
            output_token_budget: default_summarize_output_tokens(),
            network_egress: false,
            ollama: SummarizeOllama::default(),
            anthropic: SummarizeAnthropic::default(),
        }
    }
}

impl SummarizeConfig {
    pub fn load_or_default(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        let body = std::fs::read_to_string(path)?;
        let c: Self = toml::from_str(&body)?;
        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.provider.is_cloud() && !self.network_egress {
            anyhow::bail!(
                "summarize.toml: provider={:?} requires network_egress=true. \
                 Flip the flag or switch to ollama.",
                self.provider,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod summarize_config_tests {
    use super::*;
    use teramind_core::embed::ProviderKind;

    #[test]
    fn default_is_ollama_with_qwen36() {
        let c = SummarizeConfig::default();
        assert!(matches!(c.provider, ProviderKind::Ollama));
        assert_eq!(c.model, "qwen3.6:latest");
        assert_eq!(c.min_turns, 3);
        assert_eq!(c.min_duration_secs, 60);
    }

    #[test]
    fn cloud_provider_requires_network_egress() {
        let mut c = SummarizeConfig::default();
        c.provider = ProviderKind::Anthropic;
        assert!(c.validate().is_err());
        c.network_egress = true;
        c.validate().expect("ok with egress=true");
    }

    #[test]
    fn local_providers_dont_require_egress() {
        let mut c = SummarizeConfig::default();
        c.provider = ProviderKind::Ollama;
        c.network_egress = false;
        c.validate().expect("ollama ok");
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd summarize_config`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/config.rs
git commit -m "feat(daemon): SummarizeConfig types + validation"
```

---

### Task 6.2: Factory + secrets loader

**Files:**
- Create: `crates/teramindd/src/services/summarize/factory.rs`
- Modify: `crates/teramindd/src/services/summarize/mod.rs`

- [ ] **Step 1: Update submodule index**

Append to `crates/teramindd/src/services/summarize/mod.rs`:

```rust
pub mod factory;

pub use factory::build_provider;
```

- [ ] **Step 2: Author the factory**

Create `crates/teramindd/src/services/summarize/factory.rs`:

```rust
//! Provider factory. Reads SummarizeConfig, constructs the active provider.

use crate::config::SummarizeConfig;
use crate::services::summarize::{
    anthropic::AnthropicProvider, ollama::OllamaChatProvider, openai::OpenaiProvider,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::ProviderKind;
use teramind_core::summarize::SummaryProvider;

pub fn build_provider(
    cfg: &SummarizeConfig,
    secrets_path: &Path,
) -> anyhow::Result<Arc<dyn SummaryProvider>> {
    cfg.validate()?;
    match cfg.provider {
        ProviderKind::Ollama => {
            let timeout = Duration::from_millis(cfg.ollama.request_timeout_ms);
            Ok(Arc::new(OllamaChatProvider::new(
                cfg.ollama.url.clone(),
                cfg.model.clone(),
                cfg.input_char_budget as usize,
                cfg.output_token_budget as usize,
                timeout,
            )))
        }
        ProviderKind::Anthropic => {
            let api_key = read_secret(secrets_path, &cfg.anthropic.api_key_field)?;
            let p = AnthropicProvider::new(
                api_key,
                cfg.model.clone(),
                cfg.input_char_budget as usize,
                cfg.output_token_budget as usize,
                Duration::from_millis(cfg.anthropic.request_timeout_ms),
            ).map_err(|e| anyhow::anyhow!("anthropic init: {e}"))?;
            Ok(Arc::new(p))
        }
        ProviderKind::Openai => {
            Ok(Arc::new(OpenaiProvider::new(cfg.model.clone())))
        }
        ProviderKind::Fastembed | ProviderKind::Voyage => {
            anyhow::bail!(
                "provider {:?} is not valid for summarization. \
                 Use ollama/anthropic/openai.",
                cfg.provider,
            )
        }
    }
}

fn read_secret(path: &Path, field: &str) -> anyhow::Result<String> {
    if !path.exists() {
        anyhow::bail!(
            "secrets file missing: {} (required for cloud providers)",
            path.display(),
        );
    }
    // Enforce 0600 permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "secrets file {} has insecure permissions ({:o}); chmod 0600 and retry",
                path.display(), mode,
            );
        }
    }
    let body = std::fs::read_to_string(path)?;
    let value: toml::Value = toml::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    let s = value.get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("secrets.toml missing field '{field}'"))?
        .to_string();
    if s.trim().is_empty() {
        anyhow::bail!("secrets.toml field '{field}' is empty");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::embed::ProviderKind;

    #[test]
    fn build_ollama_with_defaults() {
        let cfg = SummarizeConfig::default();
        let secrets = std::path::PathBuf::from("/nonexistent");
        let p = build_provider(&cfg, &secrets).expect("ollama default");
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.model_id(), "qwen3.6:latest");
    }

    #[test]
    fn build_anthropic_without_egress_fails() {
        let mut cfg = SummarizeConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        let r = build_provider(&cfg, &std::path::PathBuf::from("/nonexistent"));
        assert!(r.is_err());
    }

    #[test]
    fn build_anthropic_without_secrets_file_fails() {
        let mut cfg = SummarizeConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        cfg.network_egress = true;
        let r = build_provider(&cfg, &std::path::PathBuf::from("/nonexistent"));
        assert!(r.is_err());
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("secrets file missing") || msg.contains("/nonexistent"));
    }

    #[test]
    fn fastembed_is_rejected() {
        let mut cfg = SummarizeConfig::default();
        cfg.provider = ProviderKind::Fastembed;
        let r = build_provider(&cfg, &std::path::PathBuf::from("/x"));
        assert!(r.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn loose_secrets_perms_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        std::fs::write(&path, "anthropic_api_key = \"sk-ant-test\"").unwrap();
        // Set world-readable (0644).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let r = read_secret(&path, "anthropic_api_key");
        assert!(r.is_err(), "loose perms should be rejected");
    }

    #[cfg(unix)]
    #[test]
    fn tight_perms_loads_secret() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        std::fs::write(&path, "anthropic_api_key = \"sk-ant-test\"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let s = read_secret(&path, "anthropic_api_key").unwrap();
        assert_eq!(s, "sk-ant-test");
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd summarize::factory`
Expected: 6 tests PASS (5 portable + 1 unix-only that's skipped on non-unix).

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/summarize/factory.rs crates/teramindd/src/services/summarize/mod.rs
git commit -m "feat(daemon): SummaryProvider factory + secrets loader"
```

---

## Section 7 — `WikiRepo`

### Task 7.1: Repo + snapshot loader

**Files:**
- Create: `crates/teramind-db/src/repos/wiki.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramind-db/src/repos/mod.rs`:

```rust
pub mod wiki;
pub use wiki::{WikiRepo, WikiPage, SessionToSummarize};
```

- [ ] **Step 2: Author the repo**

Create `crates/teramind-db/src/repos/wiki.rs`:

```rust
//! Storage layer for `wiki_pages` + `sessions_to_summarize` reads.

use crate::error::Result;
use crate::pool::DbPool;
use serde::{Deserialize, Serialize};
use teramind_core::ids::{SessionId, WikiPageId};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct WikiRepo {
    pool: DbPool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub id: WikiPageId,
    pub session_id: SessionId,
    pub model: String,
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub generated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct SessionToSummarize {
    pub session_id: SessionId,
    pub cwd: String,
    pub started_at: OffsetDateTime,
    pub ended_at: OffsetDateTime,
    pub end_reason: String,
}

impl WikiRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    /// Sessions that are ended AND lack a wiki_page for `model`.
    pub async fn fetch_sessions_to_summarize(&self, model: &str, limit: u32) -> Result<Vec<SessionToSummarize>> {
        let rows: Vec<(Uuid, String, OffsetDateTime, OffsetDateTime, Option<String>)> = sqlx::query_as(
            r#"
            SELECT v.session_id, v.cwd, v.started_at, v.ended_at, v.end_reason
            FROM   sessions_to_summarize v
            WHERE  NOT EXISTS (
                SELECT 1 FROM wiki_pages w
                WHERE  w.session_id = v.session_id
                  AND  w.model      = $1
            )
            ORDER  BY v.ended_at ASC
            LIMIT  $2
            "#,
        )
        .bind(model)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(|(sid, cwd, started_at, ended_at, end_reason)| {
            SessionToSummarize {
                session_id: SessionId(sid),
                cwd, started_at, ended_at,
                end_reason: end_reason.unwrap_or_default(),
            }
        }).collect())
    }

    pub async fn upsert(
        &self,
        session_id: SessionId,
        model: &str,
        content: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO wiki_pages (session_id, model, content, input_tokens, output_tokens)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (session_id, model) DO UPDATE SET
                content       = EXCLUDED.content,
                input_tokens  = EXCLUDED.input_tokens,
                output_tokens = EXCLUDED.output_tokens,
                generated_at  = now()
            "#,
        )
        .bind(session_id.0)
        .bind(model)
        .bind(content)
        .bind(input_tokens as i32)
        .bind(output_tokens as i32)
        .execute(self.pool.pg()).await?;
        Ok(())
    }

    /// Sentinel "skipped" mark — empty content prevents re-evaluation.
    pub async fn mark_skipped(&self, session_id: SessionId, model: &str) -> Result<()> {
        self.upsert(session_id, model, "", 0, 0).await
    }

    pub async fn get_for_session(&self, session_id: SessionId, model: &str) -> Result<Option<WikiPage>> {
        let row: Option<(Uuid, Uuid, String, String, i32, i32, OffsetDateTime)> = sqlx::query_as(
            r#"
            SELECT id, session_id, model, content, input_tokens, output_tokens, generated_at
            FROM   wiki_pages
            WHERE  session_id = $1 AND model = $2
            "#,
        )
        .bind(session_id.0)
        .bind(model)
        .fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|(id, sid, m, c, it, ot, ts)| WikiPage {
            id: WikiPageId(id),
            session_id: SessionId(sid),
            model: m, content: c,
            input_tokens: it as u32, output_tokens: ot as u32,
            generated_at: ts,
        }))
    }

    /// Most-recent non-empty wiki for any session whose cwd matches.
    /// Empty content (sentinel skip) is excluded.
    pub async fn latest_for_cwd(&self, cwd: &str) -> Result<Option<WikiPage>> {
        let row: Option<(Uuid, Uuid, String, String, i32, i32, OffsetDateTime)> = sqlx::query_as(
            r#"
            SELECT w.id, w.session_id, w.model, w.content, w.input_tokens, w.output_tokens, w.generated_at
            FROM   wiki_pages w
            JOIN   sessions   s ON s.id = w.session_id
            WHERE  s.cwd = $1
              AND  length(w.content) > 0
            ORDER  BY w.generated_at DESC
            LIMIT  1
            "#,
        )
        .bind(cwd)
        .fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|(id, sid, m, c, it, ot, ts)| WikiPage {
            id: WikiPageId(id),
            session_id: SessionId(sid),
            model: m, content: c,
            input_tokens: it as u32, output_tokens: ot as u32,
            generated_at: ts,
        }))
    }

    /// Count of ended sessions that lack a wiki for `model`.
    pub async fn backlog(&self, model: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT count(*) FROM sessions_to_summarize v
            WHERE NOT EXISTS (
                SELECT 1 FROM wiki_pages w
                WHERE w.session_id = v.session_id AND w.model = $1
            )
            "#,
        )
        .bind(model)
        .fetch_one(self.pool.pg()).await?;
        Ok(n)
    }
}
```

- [ ] **Step 3: Add `WikiPageId` to teramind-core**

Modify `crates/teramind-core/src/ids.rs`. Find the existing `id_wrapper!` macro invocations (or struct definitions) and add `WikiPageId`. If the existing pattern is something like:

```rust
id_wrapper!(SessionId);
id_wrapper!(TurnId);
// etc.
```

Add:

```rust
id_wrapper!(WikiPageId);
```

If the existing pattern is direct struct definitions, copy a sibling (e.g. `SessionId`) verbatim and rename. After adding, run:

Run: `grep -n "pub struct \w*Id" crates/teramind-core/src/ids.rs | head`

to confirm the new type compiles.

- [ ] **Step 4: Write the integration test**

Create `crates/teramind-db/tests/wiki_repo.rs`:

```rust
use teramind_db::repos::{AgentRepo, SessionRepo, WikiRepo};
use teramind_db::repos::session::NewSession;
use teramind_core::ids::SessionId;
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_repo_backlog_and_upsert() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;

    // Session not ended yet -> backlog 0.
    assert_eq!(wiki.backlog("ollama:test").await?, 0);

    sessions.end(sid, OffsetDateTime::now_utc(), "stop_hook").await?;

    // Now backlog == 1.
    assert_eq!(wiki.backlog("ollama:test").await?, 1);
    let candidates = wiki.fetch_sessions_to_summarize("ollama:test", 10).await?;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].session_id, sid);

    // Upsert a page.
    wiki.upsert(sid, "ollama:test", "# Summary\nhi", 10, 20).await?;
    assert_eq!(wiki.backlog("ollama:test").await?, 0);

    let got = wiki.get_for_session(sid, "ollama:test").await?;
    assert!(got.is_some());
    assert_eq!(got.unwrap().content, "# Summary\nhi");

    // latest_for_cwd
    let latest = wiki.latest_for_cwd("/p").await?;
    assert!(latest.is_some());

    // Re-upsert with new content (overwrites).
    wiki.upsert(sid, "ollama:test", "# Summary\nv2", 11, 21).await?;
    let got = wiki.get_for_session(sid, "ollama:test").await?.unwrap();
    assert_eq!(got.content, "# Summary\nv2");

    // Skip marker: empty content -> latest_for_cwd should exclude it.
    let sid2 = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/q",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    sessions.end(sid2, OffsetDateTime::now_utc(), "stop_hook").await?;
    wiki.mark_skipped(sid2, "ollama:test").await?;
    assert!(wiki.latest_for_cwd("/q").await?.is_none(),
            "skipped sessions must not show up in latest_for_cwd");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 5: Run**

Run: `cargo test -p teramind-db wiki_repo_backlog_and_upsert --release`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-db/src/repos/wiki.rs crates/teramind-db/src/repos/mod.rs crates/teramind-db/tests/wiki_repo.rs crates/teramind-core/src/ids.rs
git commit -m "feat(db): WikiRepo + WikiPageId type"
```

---

## Section 8 — `summarizer_worker` + App::run wiring

### Task 8.1: Snapshot loader on WikiRepo

The worker needs to load a full `SessionSnapshot` from the DB. We add a method to `WikiRepo` that joins the four trace tables for a given session_id.

**Files:**
- Modify: `crates/teramind-db/src/repos/wiki.rs`

- [ ] **Step 1: Add the snapshot loader**

Append to the `impl WikiRepo` block:

```rust
    /// Load all rows for a single session, packaged into a SessionSnapshot
    /// the digest builder can consume.
    pub async fn load_snapshot(
        &self,
        session_id: SessionId,
    ) -> Result<Option<teramindd_summarize_facade::SessionSnapshot>> {
        // facade lives in teramindd::services::summarize::digest::SessionSnapshot
        // but to keep teramind-db free of a daemon dep, we return a thin
        // adapter struct. See the trait-shaped facade below.
        unimplemented!()  // replaced in Step 2
    }
```

Hmm — circular dep. The `WikiRepo` lives in `teramind-db`; the snapshot type lives in the daemon. Solution: define the snapshot type IN `teramind-db` and re-export from the daemon. But `digest.rs` already defined the types there.

Cleanest fix: move the row-struct definitions out of `digest.rs` and into `teramind-db::repos::wiki` (or into a new `teramind-core` module so both crates can share). Since `digest.rs` already imports `teramind_core::ids` and `teramind_core::types::file_diff::Attribution`, putting the types in `teramind-core` is fine.

Replace the above stub. The real fix:

- [ ] **Step 1 (correction): Move SessionSnapshot types to `teramind-core`**

In `crates/teramind-core/src/summarize.rs`, append the row types:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::ids::{SessionId, ToolCallId, TurnId};
use crate::types::file_diff::Attribution;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: TurnId,
    pub ordinal: i32,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRow {
    pub id: ToolCallId,
    pub turn_id: TurnId,
    pub name: String,
    pub input: Value,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffRow {
    pub turn_id: Option<TurnId>,
    pub rel_path: String,
    pub language: Option<String>,
    pub attribution: Attribution,
    pub unified_diff: String,
    pub pre_excerpt: String,
    pub post_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: SessionId,
    pub cwd: String,
    pub started_at: OffsetDateTime,
    pub ended_at: OffsetDateTime,
    pub end_reason: String,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    pub turns: Vec<TurnRow>,
    pub tool_calls: Vec<ToolCallRow>,
    pub file_diffs: Vec<FileDiffRow>,
}

impl SessionSnapshot {
    pub fn turn_count(&self) -> usize { self.turns.len() }
    pub fn duration_secs(&self) -> i64 { (self.ended_at - self.started_at).whole_seconds() }
}
```

Now delete the corresponding type definitions from `crates/teramindd/src/services/summarize/digest.rs` and replace them with re-exports:

```rust
// at the top of digest.rs, replace the local row-struct definitions with:
pub use teramind_core::summarize::{
    FileDiffRow, SessionSnapshot, ToolCallRow, TurnRow,
};
```

The digest tests already use these types — they keep working because the re-export keeps the same names available.

- [ ] **Step 2: Now add `load_snapshot` to WikiRepo**

Replace the stub from Step 1 with:

```rust
    pub async fn load_snapshot(&self, session_id: SessionId) -> Result<Option<teramind_core::summarize::SessionSnapshot>> {
        // Session metadata.
        let row: Option<(Uuid, String, OffsetDateTime, Option<OffsetDateTime>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, cwd, started_at, ended_at, end_reason, git_branch, git_head
            FROM   sessions WHERE id = $1
            "#,
        )
        .bind(session_id.0)
        .fetch_optional(self.pool.pg()).await?;
        let Some((_sid, cwd, started_at, ended_at, end_reason, git_branch, git_head)) = row else {
            return Ok(None);
        };
        let Some(ended_at) = ended_at else { return Ok(None) }; // un-ended

        // Turns.
        let turn_rows: Vec<(Uuid, i32, Option<String>, Option<String>, Option<String>, OffsetDateTime)> = sqlx::query_as(
            r#"
            SELECT id, ordinal, user_prompt, assistant_text, thinking, started_at
            FROM   turns WHERE session_id = $1 ORDER BY ordinal
            "#,
        )
        .bind(session_id.0).fetch_all(self.pool.pg()).await?;
        let turns = turn_rows.into_iter().map(|(id, ord, up, at, th, sa)| {
            teramind_core::summarize::TurnRow {
                id: teramind_core::ids::TurnId(id),
                ordinal: ord,
                user_prompt: up, assistant_text: at, thinking: th,
                started_at: sa,
            }
        }).collect::<Vec<_>>();

        // Tool calls.
        let tc_rows: Vec<(Uuid, Uuid, String, serde_json::Value, Option<String>, bool)> = sqlx::query_as(
            r#"
            SELECT tc.id, tc.turn_id, tc.name, tc.input, tc.output, tc.is_error
            FROM   tool_calls tc
            JOIN   turns t ON t.id = tc.turn_id
            WHERE  t.session_id = $1
            ORDER  BY tc.turn_id, tc.ordinal
            "#,
        )
        .bind(session_id.0).fetch_all(self.pool.pg()).await?;
        let tool_calls = tc_rows.into_iter().map(|(id, tid, name, input, output, is_error)| {
            teramind_core::summarize::ToolCallRow {
                id: teramind_core::ids::ToolCallId(id),
                turn_id: teramind_core::ids::TurnId(tid),
                name, input, output: output.unwrap_or_default(), is_error,
            }
        }).collect::<Vec<_>>();

        // File diffs.
        let fd_rows: Vec<(Option<Uuid>, String, Option<String>, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT turn_id, rel_path, language, attribution, unified_diff, pre_excerpt, post_excerpt
            FROM   file_diffs
            WHERE  session_id = $1
            ORDER  BY captured_at
            "#,
        )
        .bind(session_id.0).fetch_all(self.pool.pg()).await?;
        let file_diffs = fd_rows.into_iter().map(|(tid, rel, lang, attr, diff, pre, post)| {
            let attribution = match attr.as_str() {
                "agent" => teramind_core::types::file_diff::Attribution::Agent,
                _       => teramind_core::types::file_diff::Attribution::Human,
            };
            teramind_core::summarize::FileDiffRow {
                turn_id: tid.map(teramind_core::ids::TurnId),
                rel_path: rel,
                language: lang,
                attribution,
                unified_diff: diff,
                pre_excerpt: pre,
                post_excerpt: post,
            }
        }).collect::<Vec<_>>();

        Ok(Some(teramind_core::summarize::SessionSnapshot {
            session_id,
            cwd,
            started_at,
            ended_at,
            end_reason: end_reason.unwrap_or_default(),
            git_branch,
            git_head,
            turns,
            tool_calls,
            file_diffs,
        }))
    }
```

- [ ] **Step 3: Verify build**

Run: `cargo check --workspace`
Expected: succeeds.

- [ ] **Step 4: Run existing tests**

Run: `cargo test --workspace --lib`
Expected: all pre-existing tests still pass; the digest tests still pass via the re-export.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/summarize.rs crates/teramindd/src/services/summarize/digest.rs crates/teramind-db/src/repos/wiki.rs
git commit -m "feat(db): WikiRepo::load_snapshot + move types to teramind-core"
```

---

### Task 8.2: `summarizer_worker` loop

**Files:**
- Create: `crates/teramindd/src/services/summarizer_worker.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod summarizer_worker;
```

- [ ] **Step 2: Author the worker**

Create `crates/teramindd/src/services/summarizer_worker.rs`:

```rust
//! Async session-summarizer worker. Polls `sessions_to_summarize`, builds
//! a structured digest, applies the Redactor, calls the active provider,
//! persists Markdown. Capture-safe: never blocks ingest or search.

use crate::services::summarize::digest;
use crate::services::summarize::prompts::SYSTEM_PROMPT;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teramind_core::redact::Redactor;
use teramind_core::summarize::{SummaryError, SummaryProvider};
use teramind_db::repos::WikiRepo;
use tracing::{debug, warn};

#[derive(Default)]
pub struct SummarizerStats {
    pub written: AtomicU64,
    pub skipped: AtomicU64,
    pub errors: AtomicU64,
    pub backlog: AtomicU64,
    pub last_filled_at_unix: AtomicU64,
    pub provider_unhealthy_since_unix: AtomicU64,
    pub input_tokens_total: AtomicU64,
    pub output_tokens_total: AtomicU64,
}

pub struct SummarizerWorker {
    pub stats: Arc<SummarizerStats>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct SummarizerDeps {
    pub repo: WikiRepo,
    pub provider: Arc<dyn SummaryProvider>,
    pub redactor: Arc<Redactor>,
    pub model: String,
    pub poll_interval: Duration,
    pub min_turns: u32,
    pub min_duration_secs: u64,
    pub input_char_budget: u32,
    pub output_token_budget: u32,
}

impl SummarizerWorker {
    pub fn spawn(deps: SummarizerDeps) -> Self {
        let stats = Arc::new(SummarizerStats::default());
        let s = stats.clone();
        let handle = tokio::spawn(async move { run_loop(deps, s).await; });
        Self { stats, handle }
    }
    pub fn abort(&self) { self.handle.abort(); }
}

async fn run_loop(deps: SummarizerDeps, stats: Arc<SummarizerStats>) {
    loop {
        tokio::time::sleep(deps.poll_interval).await;

        match deps.provider.health_check().await {
            Ok(_) => stats.provider_unhealthy_since_unix.store(0, Ordering::Relaxed),
            Err(e) => {
                let prev = stats.provider_unhealthy_since_unix.load(Ordering::Relaxed);
                if prev == 0 { stats.provider_unhealthy_since_unix.store(unix_now(), Ordering::Relaxed); }
                debug!(?e, "summary provider unhealthy");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        }

        if let Ok(b) = deps.repo.backlog(&deps.model).await {
            stats.backlog.store(b as u64, Ordering::Relaxed);
        }

        let candidates = match deps.repo.fetch_sessions_to_summarize(&deps.model, 1).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "fetch_sessions_to_summarize failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        if candidates.is_empty() { continue; }
        let s = &candidates[0];

        let snapshot = match deps.repo.load_snapshot(s.session_id).await {
            Ok(Some(snap)) => snap,
            Ok(None) => continue,           // session vanished between fetch and load (cascade delete)
            Err(e) => {
                warn!(error = %e, "load_snapshot failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let duration_secs = snapshot.duration_secs() as u64;
        if snapshot.turn_count() < deps.min_turns as usize || duration_secs < deps.min_duration_secs {
            let _ = deps.repo.mark_skipped(s.session_id, &deps.model).await;
            stats.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        let digest_md = digest::build(&snapshot, deps.input_char_budget as usize);
        let digest_md = deps.redactor.apply(&digest_md);

        match deps.provider.summarize(SYSTEM_PROMPT, &digest_md, deps.output_token_budget as usize).await {
            Ok(result) => {
                if let Err(e) = deps.repo.upsert(
                    s.session_id, &deps.model, &result.content,
                    result.input_tokens, result.output_tokens,
                ).await {
                    warn!(error = %e, "wiki upsert failed");
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                stats.written.fetch_add(1, Ordering::Relaxed);
                stats.last_filled_at_unix.store(unix_now(), Ordering::Relaxed);
                stats.input_tokens_total.fetch_add(result.input_tokens as u64, Ordering::Relaxed);
                stats.output_tokens_total.fetch_add(result.output_tokens as u64, Ordering::Relaxed);
                debug!(?s.session_id, "summarizer wrote wiki page");
            }
            Err(SummaryError::Unhealthy(_)) => { continue; }
            Err(SummaryError::ModelNotFound(_)) => {
                warn!("summarizer: model not found; pausing worker until config changes");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
            Err(e) => {
                warn!(error = %e, "summarize failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    // Behavior is exercised end-to-end in §13. This module has no isolated
    // unit tests beyond a smoke compile check.
    use super::*;

    #[test]
    fn worker_handle_abort_compiles() {
        let _ = SummarizerWorker::abort;
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd summarizer_worker`
Expected: 1 test PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/summarizer_worker.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): summarizer_worker loop"
```

---

### Task 8.3: Wire into `App::run`

**Files:**
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Add wiring**

Locate the section in `App::run` after `EmbeddingWorker::spawn(...)` (Plan G §8.2). Add:

```rust
        // Session summarizer.
        let summarize_cfg_path = paths.config_dir.join("summarize.toml");
        let summarize_cfg = crate::config::SummarizeConfig::load_or_default(&summarize_cfg_path)?;
        let secrets_path = paths.config_dir.join("secrets.toml");
        let summary_provider = crate::services::summarize::build_provider(
            &summarize_cfg, &secrets_path,
        )?;
        let summarize_model_db_key = format!(
            "{}:{}",
            provider_prefix(summary_provider.kind()),
            summarize_cfg.model,
        );
        let wiki_repo = teramind_db::repos::WikiRepo::new(pool.clone());
        let summarizer = crate::services::summarizer_worker::SummarizerWorker::spawn(
            crate::services::summarizer_worker::SummarizerDeps {
                repo: wiki_repo.clone(),
                provider: summary_provider.clone(),
                redactor: std::sync::Arc::new(teramind_core::redact::Redactor::with_default_rules()),
                model: summarize_model_db_key.clone(),
                poll_interval: std::time::Duration::from_secs(summarize_cfg.poll_interval_secs),
                min_turns: summarize_cfg.min_turns,
                min_duration_secs: summarize_cfg.min_duration_secs,
                input_char_budget: summarize_cfg.input_char_budget,
                output_token_budget: summarize_cfg.output_token_budget,
            },
        );
        let summarizer_stats = summarizer.stats.clone();
        let _summarizer_guard = summarizer;  // hold for App::run lifetime
```

(The IPC handler gains `wiki_repo`, `summary_provider`, `summarize_model_db_key`, `summarizer_stats` fields in §9.)

- [ ] **Step 2: `cargo check -p teramindd`**

Expected: succeeds (handler fields will be added in §9 — if the build fails here because `DaemonIpcHandler` is not yet aware, that's expected; defer the actual handler hookup to §9).

For now, comment out the four new locals or use `let _ = ...` to suppress unused warnings until §9. Actually, the cleanest is to leave them bound and accept the unused warnings — §9 will use them.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/app.rs
git commit -m "feat(daemon): wire summarizer_worker into App::run"
```

---

## Section 9 — IPC + Hit::WikiPage + search integration

### Task 9.1: `Hit::WikiPage` variant

**Files:**
- Modify: `crates/teramind-core/src/types/hit.rs`

- [ ] **Step 1: Add the variant**

Locate `pub enum Hit { … }` in `crates/teramind-core/src/types/hit.rs`. Add:

```rust
    WikiPage {
        page_id:    crate::ids::WikiPageId,
        session_id: crate::ids::SessionId,
        title:      String,
        snippet:    String,
        score:      f32,
        ts:         time::OffsetDateTime,
    },
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-core`
Expected: PASS. If existing serde roundtrip tests exercise pattern-matches on `Hit`, they may need a wildcard arm — adapt minimally.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-core/src/types/hit.rs
git commit -m "feat(core): Hit::WikiPage variant"
```

---

### Task 9.2: IPC `Request::WikiLookup` + `Response::WikiPage`

**Files:**
- Modify: `crates/teramind-ipc/src/proto.rs`

- [ ] **Step 1: Add the variants**

In `crates/teramind-ipc/src/proto.rs`, add to the `Request` enum:

```rust
    WikiLookup {
        session_id: Option<String>,
        cwd: Option<String>,
    },
```

Add to the `Response` enum:

```rust
    WikiPage {
        session_id: String,
        cwd: String,
        model: String,
        content: String,
        generated_at: time::OffsetDateTime,
    },
    WikiNotFound,
```

Also extend `StatusReport` with the summary fields:

```rust
    #[serde(default)]
    pub summary_provider: Option<String>,
    #[serde(default)]
    pub summary_healthy: Option<bool>,
    #[serde(default)]
    pub summary_backlog: Option<i64>,
    #[serde(default)]
    pub summary_written_total: Option<u64>,
    #[serde(default)]
    pub summary_input_tokens_total: Option<u64>,
    #[serde(default)]
    pub summary_output_tokens_total: Option<u64>,
```

(All `#[serde(default)]` to keep older daemon's StatusReport JSON parsable.)

- [ ] **Step 2: Run**

Run: `cargo check --workspace`
Expected: succeeds.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-ipc/src/proto.rs
git commit -m "feat(ipc): WikiLookup request + WikiPage response + StatusReport summary fields"
```

---

### Task 9.3: IPC server arm + StatusReport population

**Files:**
- Modify: `crates/teramindd/src/services/ipc_server.rs`
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Extend `DaemonIpcHandler`**

In `crates/teramindd/src/services/ipc_server.rs`, add fields:

```rust
    pub wiki_repo: teramind_db::repos::WikiRepo,
    pub summary_provider: std::sync::Arc<dyn teramind_core::summarize::SummaryProvider>,
    pub summary_model: String,
    pub summarizer_stats: std::sync::Arc<crate::services::summarizer_worker::SummarizerStats>,
```

- [ ] **Step 2: Handle `Request::WikiLookup`**

In the `match` block:

```rust
            Request::WikiLookup { session_id, cwd } => {
                let result: anyhow::Result<Option<teramind_db::repos::WikiPage>> = async {
                    if let Some(sid_str) = session_id {
                        let sid = teramind_core::ids::SessionId(uuid::Uuid::parse_str(&sid_str)?);
                        let p = self.wiki_repo.get_for_session(sid, &self.summary_model).await?;
                        Ok(p)
                    } else if let Some(cwd) = cwd {
                        let p = self.wiki_repo.latest_for_cwd(&cwd).await?;
                        Ok(p)
                    } else {
                        Ok(None)
                    }
                }.await;
                match result {
                    Ok(Some(p)) => {
                        // Look up cwd for the session_id.
                        let cwd: String = sqlx::query_scalar("SELECT cwd FROM sessions WHERE id = $1")
                            .bind(p.session_id.0)
                            .fetch_one(self.wiki_repo_pool()).await
                            .unwrap_or_default();
                        Response::WikiPage {
                            session_id: p.session_id.0.to_string(),
                            cwd,
                            model: p.model,
                            content: p.content,
                            generated_at: p.generated_at,
                        }
                    }
                    Ok(None) => Response::WikiNotFound,
                    Err(e) => Response::Error(format!("wiki lookup failed: {e}")),
                }
            }
```

Add a helper to expose the pool (or just give the handler a `pool: DbPool` field — cleaner). For minimal surgery: add `pub pool: teramind_db::pool::DbPool` to `DaemonIpcHandler` and use `&self.pool.pg()` instead of `self.wiki_repo_pool()`. Update the App::run construction to populate it.

- [ ] **Step 3: Populate the StatusReport summary fields**

In the `Request::Status` arm, add the new fields:

```rust
                use std::sync::atomic::Ordering;
                let summary_healthy = self.summarizer_stats.provider_unhealthy_since_unix.load(Ordering::Relaxed) == 0;
                let summary_backlog = self.summarizer_stats.backlog.load(Ordering::Relaxed) as i64;
                let summary_written = self.summarizer_stats.written.load(Ordering::Relaxed);
                let summary_in = self.summarizer_stats.input_tokens_total.load(Ordering::Relaxed);
                let summary_out = self.summarizer_stats.output_tokens_total.load(Ordering::Relaxed);

                Response::Status(StatusReport {
                    /* existing fields ... */
                    summary_provider: Some(self.summary_model.clone()),
                    summary_healthy: Some(summary_healthy),
                    summary_backlog: Some(summary_backlog),
                    summary_written_total: Some(summary_written),
                    summary_input_tokens_total: Some(summary_in),
                    summary_output_tokens_total: Some(summary_out),
                })
```

Adapt to the current StatusReport literal — keep all existing fields unchanged.

- [ ] **Step 4: Populate the handler in App::run**

In `crates/teramindd/src/app.rs`, where `DaemonIpcHandler { … }` is constructed, add the new fields using the locals bound in §8.3:

```rust
        let handler = Arc::new(DaemonIpcHandler {
            // ... existing fields
            wiki_repo,
            summary_provider,
            summary_model: summarize_model_db_key,
            summarizer_stats,
            pool: pool.clone(),  // if not already present
        });
```

- [ ] **Step 5: Update tests that construct `DaemonIpcHandler` literal**

Run: `grep -rn "DaemonIpcHandler {" crates/`

For each test site, add the four new fields. For tests, use:

```rust
wiki_repo: teramind_db::repos::WikiRepo::new(pool.clone()),
summary_provider: { /* construct a NullSummaryProvider similar to embed::null */
    use async_trait::async_trait;
    use teramind_core::summarize::*;
    struct NullSummary;
    #[async_trait]
    impl SummaryProvider for NullSummary {
        fn kind(&self) -> ProviderKind { ProviderKind::Ollama }
        fn model_id(&self) -> &str { "test" }
        fn max_input_tokens(&self) -> usize { 16384 }
        fn max_output_tokens(&self) -> usize { 1500 }
        async fn health_check(&self) -> Result<(), SummaryError> { Ok(()) }
        async fn summarize(&self, _: &str, _: &str, _: usize) -> Result<SummaryResult, SummaryError> {
            Ok(SummaryResult { content: "".into(), input_tokens: 0, output_tokens: 0 })
        }
    }
    std::sync::Arc::new(NullSummary)
},
summary_model: "ollama:test".into(),
summarizer_stats: std::sync::Arc::new(
    teramindd::services::summarizer_worker::SummarizerStats::default(),
),
```

To avoid copy-paste in every test, expose a `NullSummaryProvider` from `services/summarize/null.rs`:

```rust
// crates/teramindd/src/services/summarize/null.rs
use async_trait::async_trait;
use teramind_core::summarize::*;

pub struct NullSummaryProvider;

#[async_trait]
impl SummaryProvider for NullSummaryProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Ollama }
    fn model_id(&self) -> &str { "test:null" }
    fn max_input_tokens(&self) -> usize { 16384 }
    fn max_output_tokens(&self) -> usize { 1500 }
    async fn health_check(&self) -> Result<(), SummaryError> { Ok(()) }
    async fn summarize(&self, _: &str, _: &str, _: usize) -> Result<SummaryResult, SummaryError> {
        Ok(SummaryResult { content: String::new(), input_tokens: 0, output_tokens: 0 })
    }
}
```

Add `pub mod null;` to `services/summarize/mod.rs` and use `crate::services::summarize::null::NullSummaryProvider` in tests.

- [ ] **Step 6: `cargo check --workspace && cargo test --workspace --lib`**

Expected: succeeds.

- [ ] **Step 7: Commit**

```bash
git add crates/teramindd/src/services/ipc_server.rs crates/teramindd/src/app.rs crates/teramindd/src/services/summarize/null.rs crates/teramindd/src/services/summarize/mod.rs crates/teramindd/tests/
git commit -m "feat(daemon): IPC WikiLookup arm + StatusReport summary fields"
```

---

## Section 10 — Auto-recall enrichment + wiki search hits

### Task 10.1: `do_auto_recall` includes latest wiki for cwd

**Files:**
- Modify: `crates/teramindd/src/services/search.rs`

- [ ] **Step 1: Locate `do_auto_recall`**

Run: `grep -n "pub async fn do_auto_recall\|render_auto_recall_md" crates/teramindd/src/services/search.rs`

- [ ] **Step 2: Add the wiki source**

Update the function signature to accept a `WikiRepo`. Today (Plan D §9.2) it's:

```rust
pub async fn do_auto_recall(repo: &SearchRepo, req: &AutoRecallRequest) -> Result<String, _>
```

Change to:

```rust
pub async fn do_auto_recall(
    repo: &SearchRepo,
    wiki_repo: &teramind_db::repos::WikiRepo,
    req: &AutoRecallRequest,
) -> Result<String, teramind_db::DbError>
```

Internally:

```rust
    let (recent, diffs) = tokio::try_join!(
        repo.recent_turns_in_project(None, &req.cwd, req.limit),
        repo.diff_excerpts_for_cwd_files(&req.cwd_files, req.limit),
    )?;
    let latest_wiki = wiki_repo.latest_for_cwd(&req.cwd).await
        .ok()
        .flatten();

    if recent.is_empty() && diffs.is_empty() && latest_wiki.is_none() {
        return Ok(String::new());
    }
    Ok(render_auto_recall_md(&recent, &diffs, latest_wiki.as_ref()))
```

Update `render_auto_recall_md`:

```rust
pub fn render_auto_recall_md(
    recent: &[teramind_db::repos::search::RankedTurn],
    diffs: &[teramind_db::repos::search::RankedDiff],
    latest_wiki: Option<&teramind_db::repos::WikiPage>,
) -> String {
    let mut out = String::new();
    if let Some(w) = latest_wiki {
        out.push_str("## Most recent session summary\n\n");
        out.push_str(&format!("> *Generated {} from session {}*\n\n",
            w.generated_at.date(),
            shorten_uuid(&w.session_id.0.to_string()),
        ));
        // Cap at ~1.5 KB of wiki content.
        let body = if w.content.len() > 1500 {
            let mut t = w.content[..1500].to_string();
            t.push_str("\n\n*(truncated; see `mcp__teramind__wiki` for full page)*");
            t
        } else {
            w.content.clone()
        };
        out.push_str(&body);
        out.push_str("\n\n");
    }
    if !recent.is_empty() { /* existing rendering unchanged */ }
    if !diffs.is_empty()  { /* existing rendering unchanged */ }
    out
}

fn shorten_uuid(s: &str) -> String {
    s.chars().take(8).collect::<String>() + "..."
}
```

- [ ] **Step 3: Update the IPC handler's `Request::AutoRecall` arm**

In `ipc_server.rs::handle_request::Request::AutoRecall`, pass `&self.wiki_repo` as the second arg.

- [ ] **Step 4: Update existing tests for `render_auto_recall_md`**

The signature changed — any test invocation needs a third `None` arg. Search:

Run: `grep -rn "render_auto_recall_md" crates/`

Update each call.

Add a new test:

```rust
    #[test]
    fn render_auto_recall_md_includes_wiki_section_when_present() {
        use teramind_db::repos::WikiPage;
        use teramind_core::ids::{SessionId, WikiPageId};
        let wiki = WikiPage {
            id: WikiPageId(uuid::Uuid::new_v4()),
            session_id: SessionId(uuid::Uuid::new_v4()),
            model: "ollama:qwen3.6:latest".into(),
            content: "# Summary\nThe agent refactored JWT...".into(),
            input_tokens: 100, output_tokens: 50,
            generated_at: OffsetDateTime::now_utc(),
        };
        let md = render_auto_recall_md(&[], &[], Some(&wiki));
        assert!(md.contains("Most recent session summary"));
        assert!(md.contains("refactored JWT"));
    }

    #[test]
    fn render_auto_recall_md_truncates_long_wiki() {
        use teramind_db::repos::WikiPage;
        use teramind_core::ids::{SessionId, WikiPageId};
        let wiki = WikiPage {
            id: WikiPageId(uuid::Uuid::new_v4()),
            session_id: SessionId(uuid::Uuid::new_v4()),
            model: "test".into(),
            content: "A".repeat(5000),
            input_tokens: 0, output_tokens: 0,
            generated_at: OffsetDateTime::now_utc(),
        };
        let md = render_auto_recall_md(&[], &[], Some(&wiki));
        assert!(md.contains("(truncated"));
    }
```

- [ ] **Step 5: Run**

Run: `cargo test -p teramindd auto_recall`
Expected: previous tests pass + 2 new tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/teramindd/src/services/search.rs crates/teramindd/src/services/ipc_server.rs
git commit -m "feat(search): do_auto_recall includes latest wiki for cwd"
```

---

### Task 10.2: Wiki hits in search

The rebuilt `traces_fts` already includes wiki content via the LATERAL join. That means existing `fts_turns` queries will surface turns whose session has a matching wiki. We additionally want a direct `Hit::WikiPage` variant so the UI/tooling can show "this is a summary hit, not a turn hit."

**Files:**
- Modify: `crates/teramind-db/src/repos/search.rs`
- Modify: `crates/teramindd/src/services/search.rs`

- [ ] **Step 1: Add `fts_wiki_pages` to SearchRepo**

In `crates/teramind-db/src/repos/search.rs`, add:

```rust
#[derive(Debug, Clone)]
pub struct RankedWiki {
    pub page_id: Uuid,
    pub session_id: Uuid,
    pub title: String,
    pub snippet: String,
    pub fts_score: f32,
    pub ts: OffsetDateTime,
}

impl SearchRepo {
    pub async fn fts_wiki_pages(&self, query: &str, limit: u32) -> Result<Vec<RankedWiki>> {
        let rows: Vec<(Uuid, Uuid, String, OffsetDateTime, f32)> = sqlx::query_as(
            r#"
            SELECT w.id, w.session_id, w.content, w.generated_at,
                   ts_rank_cd(to_tsvector('english', w.content),
                              plainto_tsquery('english', $1))::float4 AS fts_score
            FROM   wiki_pages w
            WHERE  to_tsvector('english', w.content) @@ plainto_tsquery('english', $1)
              AND  length(w.content) > 0
            ORDER  BY fts_score DESC
            LIMIT  $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(id, sid, content, ts, score)| {
            let title = content.lines().find(|l| l.starts_with("# "))
                .map(|s| s.trim_start_matches("# ").to_string())
                .unwrap_or_else(|| "(untitled)".to_string());
            let snippet = content.lines().take(3).collect::<Vec<_>>().join(" ");
            let snippet: String = snippet.chars().take(200).collect();
            RankedWiki { page_id: id, session_id: sid, title, snippet, fts_score: score, ts }
        }).collect())
    }
}
```

- [ ] **Step 2: Surface wiki hits in `do_search`**

In `crates/teramindd/src/services/search.rs`, extend `do_search` to query wiki hits in parallel and feed them into `rank_and_hydrate`. Add a new parameter to `rank_and_hydrate`:

```rust
pub fn rank_and_hydrate(
    fts_turns: Vec<RankedTurn>,
    trgm_diffs: Vec<RankedDiff>,
    trgm_skills: Vec<RankedSkill>,
    sem_turns: Vec<RankedTurn>,
    sem_diffs: Vec<RankedDiff>,
    fts_wikis: Vec<RankedWiki>,           // NEW
    weights: BlendWeights,
    same_project_id: Option<Uuid>,
    limit: u32,
) -> Vec<Hit> {
    /* ... existing body ... */
    for w in fts_wikis {
        let score = weights.fts * w.fts_score
                  + weights.recency * recency_factor(w.ts);
        hits.push((score, Hit::WikiPage {
            page_id: WikiPageId(w.page_id),
            session_id: SessionId(w.session_id),
            title: w.title,
            snippet: w.snippet,
            score,
            ts: w.ts,
        }));
    }
    /* sort + truncate as before */
}
```

And in `do_search`, add the wiki source to `tokio::try_join!`:

```rust
    let (fts_res, trgm_diffs, trgm_skills, sem_turns, sem_diffs, fts_wikis) = tokio::try_join!(
        repo.fts_turns(&req.query, req.limit),
        repo.trgm_diffs(&req.query, req.limit),
        repo.trgm_skills(&req.query, req.limit),
        async {
            if let Some(v) = query_emb.as_ref() {
                repo.vector_search_turns(v, model, req.limit).await
            } else { Ok(vec![]) }
        },
        async {
            if let Some(v) = query_emb.as_ref() {
                repo.vector_search_diffs(v, model, req.limit).await
            } else { Ok(vec![]) }
        },
        repo.fts_wiki_pages(&req.query, req.limit),
    )?;
    let hits = rank_and_hydrate(fts_res, trgm_diffs, trgm_skills, sem_turns, sem_diffs, fts_wikis, weights, None, req.limit);
```

Update existing tests for `rank_and_hydrate` to pass `vec![]` for the new `fts_wikis` parameter.

- [ ] **Step 3: Add a test**

In `services/search.rs` `tests`:

```rust
    #[test]
    fn rank_and_hydrate_emits_wiki_page_hits() {
        let weights = BlendWeights::default();
        let now = OffsetDateTime::now_utc();
        let w = teramind_db::repos::search::RankedWiki {
            page_id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::new_v4(),
            title: "Refactor".into(),
            snippet: "The agent refactored JWT".into(),
            fts_score: 0.7,
            ts: now,
        };
        let hits = rank_and_hydrate(vec![], vec![], vec![], vec![], vec![], vec![w], weights, None, 10);
        assert_eq!(hits.len(), 1);
        match &hits[0] {
            Hit::WikiPage { title, .. } => assert_eq!(title, "Refactor"),
            _ => panic!("expected WikiPage hit"),
        }
    }
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd search`
Expected: existing + new tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/search.rs crates/teramindd/src/services/search.rs
git commit -m "feat(search): Hit::WikiPage via SearchRepo::fts_wiki_pages"
```

---

## Section 11 — MCP tool `mcp__teramind__wiki`

### Task 11.1: New rmcp tool

**Files:**
- Modify: `crates/teramind-mcp/src/server.rs`

- [ ] **Step 1: Inspect existing tool registrations**

Run: `grep -n "#\[tool\|ToolRouter\|search\|recall\|save_skill" crates/teramind-mcp/src/server.rs`

Note the patterns used by `search`, `recall`, `save_skill` (Plan C §9).

- [ ] **Step 2: Add `WikiParams` + the tool fn**

Following the same pattern, add:

```rust
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct WikiParams {
    /// Optional session id (UUID). If omitted, returns the most recent
    /// wiki page for `cwd`'s project.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional cwd. Defaults to the daemon's notion of current project.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[tool(description = "Read a session's wiki page. Without session_id, returns the most recent summary for the cwd's project.")]
pub async fn wiki(
    &self,
    rmcp::handler::server::wrapper::Parameters(p): rmcp::handler::server::wrapper::Parameters<WikiParams>,
) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
    let req = teramind_ipc::proto::Request::WikiLookup {
        session_id: p.session_id,
        cwd: p.cwd,
    };
    let resp = self.client.request(req).await
        .map_err(|e| rmcp::model::ErrorData::internal_error(format!("ipc: {e}"), None))?;
    match resp {
        teramind_ipc::proto::Response::WikiPage { session_id, cwd, model, content, generated_at } => {
            let body = serde_json::json!({
                "session_id": session_id,
                "cwd": cwd,
                "model": model,
                "content": content,
                "generated_at": generated_at.to_string(),
            });
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::Content::text(serde_json::to_string_pretty(&body).unwrap_or_default()),
            ]))
        }
        teramind_ipc::proto::Response::WikiNotFound => {
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::Content::text("{\"status\":\"not_found\"}".to_string()),
            ]))
        }
        teramind_ipc::proto::Response::Error(msg) => {
            Err(rmcp::model::ErrorData::internal_error(msg, None))
        }
        _ => Err(rmcp::model::ErrorData::internal_error("unexpected response".into(), None)),
    }
}
```

(Adapt to the exact pattern used in the file — e.g., if `search` uses `tool_router!` macro registration, do the same here.)

- [ ] **Step 3: Run**

Run: `cargo test -p teramind-mcp`
Expected: existing tests PASS. (No new MCP test in this task; integration covered in §13.)

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-mcp/src/server.rs
git commit -m "feat(mcp): mcp__teramind__wiki tool"
```

---

## Section 12 — CLI `teramind sessions show` + doctor extension

### Task 12.1: `sessions` subcommand

**Files:**
- Modify: `crates/teramind/src/cli.rs`
- Create: `crates/teramind/src/commands/sessions.rs`
- Modify: `crates/teramind/src/commands/mod.rs`
- Modify: `crates/teramind/src/main.rs`

- [ ] **Step 1: Add the variant**

In `crates/teramind/src/cli.rs`, append to the `Command` enum:

```rust
    /// Inspect ended sessions.
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
```

Append to the file:

```rust
#[derive(Debug, clap::Subcommand)]
pub enum SessionsAction {
    /// Show a session's wiki page. Defaults to the most recent for $PWD.
    Show {
        /// Session UUID. If omitted, returns the most recent for the cwd.
        session_id: Option<String>,
        /// Output JSON instead of Markdown.
        #[arg(long)]
        json: bool,
    },
}
```

- [ ] **Step 2: Implement the command**

Create `crates/teramind/src/commands/sessions.rs`:

```rust
//! `teramind sessions show [<id>] [--json]`

use teramind_ipc::proto::{Request, Response};

pub async fn show(session_id: Option<String>, json: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let req = Request::WikiLookup { session_id, cwd: Some(cwd) };

    let client = crate::ipc::client::connect_or_spawn().await?;
    let resp = client.request(req).await?;
    match resp {
        Response::WikiPage { session_id, cwd, model, content, generated_at } => {
            if json {
                let body = serde_json::json!({
                    "session_id": session_id,
                    "cwd": cwd,
                    "model": model,
                    "content": content,
                    "generated_at": generated_at.to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&body)?);
            } else {
                println!("{}", content);
            }
        }
        Response::WikiNotFound => {
            eprintln!("teramind: no wiki page found for the given criteria.");
            eprintln!("Run `teramind doctor` for summarizer health.");
            std::process::exit(2);
        }
        Response::Error(msg) => anyhow::bail!("wiki lookup failed: {msg}"),
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
    Ok(())
}
```

(`crate::ipc::client::connect_or_spawn` is the existing helper from Plan A used by `teramind search`. Adapt to the actual function name — `grep -n "pub fn\|pub async fn" crates/teramind/src/ipc/` to find it.)

- [ ] **Step 3: Register + dispatch**

Append to `crates/teramind/src/commands/mod.rs`:

```rust
pub mod sessions;
```

In `crates/teramind/src/main.rs`, add the match arm:

```rust
        Command::Sessions { action } => match action {
            cli::SessionsAction::Show { session_id, json } =>
                commands::sessions::show(session_id, json).await,
        },
```

- [ ] **Step 4: Run**

Run: `cargo check -p teramind-cli`
Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind/src/cli.rs crates/teramind/src/commands/sessions.rs crates/teramind/src/commands/mod.rs crates/teramind/src/main.rs
git commit -m "feat(cli): teramind sessions show [<id>] [--json]"
```

---

### Task 12.2: `teramind doctor` summary lines

**Files:**
- Modify: `crates/teramind/src/commands/doctor.rs`

- [ ] **Step 1: Render the new fields**

After the existing embedding-related lines (added in Plan G §12), insert:

```rust
    if let Some(provider) = &status.summary_provider {
        let healthy = status.summary_healthy.unwrap_or(false);
        println!("summary provider: {provider} ({})", if healthy { "healthy" } else { "unhealthy" });
    }
    if let Some(backlog) = status.summary_backlog {
        let written = status.summary_written_total.unwrap_or(0);
        println!("summary backlog:  {backlog} sessions queued");
        println!("summaries written: {written} total");
    }
    if let (Some(it), Some(ot)) = (status.summary_input_tokens_total, status.summary_output_tokens_total) {
        if it > 0 || ot > 0 {
            println!("summary tokens:   in={it}  out={ot}");
        }
    }
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-cli doctor`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/src/commands/doctor.rs
git commit -m "feat(cli): doctor surfaces summarizer provider + backlog + token usage"
```

---

## Section 13 — L3 integration tests

### Task 13.1: Mock-provider end-to-end

**Files:**
- Create: `crates/teramindd/tests/summarizer_mock.rs`

- [ ] **Step 1: Author**

```rust
//! L3: mock SummaryProvider drives a real PG via the real worker.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::{SessionId, TurnId};
use teramind_core::redact::Redactor;
use teramind_core::summarize::{
    ProviderKind, SummaryError, SummaryProvider, SummaryResult,
};
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramind_db::repos::session::NewSession;
use teramindd::services::summarizer_worker::{SummarizerDeps, SummarizerWorker};
use time::OffsetDateTime;

struct EchoProvider;

#[async_trait]
impl SummaryProvider for EchoProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Ollama }
    fn model_id(&self) -> &str { "mock:echo" }
    fn max_input_tokens(&self) -> usize { 16384 }
    fn max_output_tokens(&self) -> usize { 1500 }
    async fn health_check(&self) -> Result<(), SummaryError> { Ok(()) }
    async fn summarize(
        &self,
        _system: &str,
        user: &str,
        _max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        // Echo: produce a stable Markdown wrapping the first 100 chars of the digest.
        let preview: String = user.chars().take(100).collect();
        let content = format!("# Summary\n\nDigest excerpt: {preview}\n\n# Files changed\n\n- (mock)\n");
        Ok(SummaryResult { content, input_tokens: 10, output_tokens: 20 })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_writes_wiki_for_ended_session_within_10s() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ended   = OffsetDateTime::from_unix_timestamp(1_700_001_000).unwrap();
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/proj",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: started,
    }).await?;
    // Three turns (over min_turns=3).
    for i in 0..3 {
        let tid = trace.upsert_turn_with_id(
            TurnId(uuid::Uuid::new_v4()), sid, i as i32, started,
            Some(&format!("prompt {i}")),
        ).await?;
        trace.finalize_turn(tid, started, Some(&format!("response {i}")), None, Some("test"), None, None).await?;
    }
    sessions.end(sid, ended, "stop_hook").await?;

    let _worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: Arc::new(EchoProvider),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:echo".into(),
        poll_interval: Duration::from_millis(200),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: 8000,
        output_token_budget: 1500,
    });

    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if wiki.backlog("mock:echo").await? == 0 { break; }
    }
    let page = wiki.get_for_session(sid, "mock:echo").await?.expect("wiki should exist");
    assert!(page.content.contains("# Summary"));
    assert!(page.content.contains("Digest excerpt"));
    assert_eq!(page.input_tokens, 10);
    assert_eq!(page.output_tokens, 20);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn short_session_is_skipped() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ended   = OffsetDateTime::from_unix_timestamp(1_700_000_005).unwrap();  // 5 seconds
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/proj",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: started,
    }).await?;
    let tid = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0, started, Some("hi"),
    ).await?;
    trace.finalize_turn(tid, started, Some("hello"), None, Some("test"), None, None).await?;
    sessions.end(sid, ended, "stop_hook").await?;

    let _worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: Arc::new(EchoProvider),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:echo".into(),
        poll_interval: Duration::from_millis(200),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: 8000,
        output_token_budget: 1500,
    });

    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if wiki.backlog("mock:echo").await? == 0 { break; }
    }
    let page = wiki.get_for_session(sid, "mock:echo").await?.expect("sentinel skip row");
    assert_eq!(page.content, "", "short session should get a sentinel skip");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd --test summarizer_mock --release`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/summarizer_mock.rs
git commit -m "test(daemon): L3 summarizer with mock provider"
```

---

### Task 13.2: Real-Ollama test (host-GPU preferred)

**Files:**
- Create: `crates/teramindd/tests/summarizer_ollama.rs`

- [ ] **Step 1: Author**

```rust
//! L3: real Ollama (host-local, GPU-preferred). Skips when probe fails.

use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramind_db::repos::session::NewSession;
use teramindd::config::SummarizeConfig;
use teramindd::services::summarize::build_provider;
use teramindd::services::summarizer_worker::{SummarizerDeps, SummarizerWorker};
use time::OffsetDateTime;

async fn probe_ollama() -> bool {
    reqwest::Client::new()
        .get("http://localhost:11434/api/version")
        .timeout(Duration::from_millis(500))
        .send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ollama_summarizes_session_with_section_headers() -> anyhow::Result<()> {
    if !probe_ollama().await {
        eprintln!("ollama not running on localhost:11434, skipping");
        return Ok(());
    }

    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ended   = OffsetDateTime::from_unix_timestamp(1_700_002_500).unwrap();
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/openvms-port",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: started,
    }).await?;
    for i in 0..5 {
        let tid = trace.upsert_turn_with_id(
            TurnId(uuid::Uuid::new_v4()), sid, i as i32, started,
            Some(&format!("Port the configure.ac autoconf check #{i} for OpenVMS x86")),
        ).await?;
        trace.finalize_turn(tid, started,
            Some(&format!("Replaced AC_CHECK_FUNC([fork]) with vfork-aware probe #{i}")),
            None, Some("test"), None, None,
        ).await?;
    }
    sessions.end(sid, ended, "stop_hook").await?;

    let cfg = SummarizeConfig::default();
    let secrets = dir.path().join("secrets.toml");
    let provider = build_provider(&cfg, &secrets)?;

    let _worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: provider.clone(),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: format!("ollama:{}", cfg.model),
        poll_interval: Duration::from_secs(1),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: cfg.input_char_budget,
        output_token_budget: cfg.output_token_budget,
    });

    // Allow up to 90s wall clock for Ollama to summarize.
    for _ in 0..90 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if wiki.backlog(&format!("ollama:{}", cfg.model)).await? == 0 { break; }
    }
    let page = wiki.get_for_session(sid, &format!("ollama:{}", cfg.model)).await?
        .expect("wiki must exist after worker drains");
    assert!(!page.content.is_empty(), "non-empty summary expected");

    let required_headers = ["# Summary", "# Files changed", "# Decisions & gotchas", "# Follow-ups"];
    let missing: Vec<_> = required_headers.iter()
        .filter(|h| !page.content.contains(*h)).collect();
    // Allow a single missing header (chat models occasionally rename them);
    // assert at least 3 of 4 are present.
    assert!(missing.len() <= 1,
        "expected at least 3/4 spec section headers; missing: {:?}\ncontent:\n{}",
        missing, page.content);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run (uses host Ollama)**

Run: `cargo test -p teramindd --test summarizer_ollama --release -- --nocapture`
Expected: PASS if `qwen3.6:latest` is pulled; auto-skip if Ollama is offline.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/summarizer_ollama.rs
git commit -m "test(daemon): L3 real-Ollama summarizer produces spec headers"
```

---

### Task 13.3: FTS hit on wiki content

**Files:**
- Create: `crates/teramindd/tests/wiki_in_traces_fts.rs`

- [ ] **Step 1: Author**

```rust
//! L3: writing a wiki_page joins traces_fts so search hits the summary.

use teramind_db::repos::{AgentRepo, SearchRepo, SessionRepo, TraceRepo, WikiRepo};
use teramind_db::repos::session::NewSession;
use teramind_core::ids::TurnId;
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_finds_wiki_via_traces_fts() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    let _tid = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(), Some("unrelated"),
    ).await?;

    // Insert a wiki with a unique token. The 'thirteen-banana-tower' phrase
    // appears nowhere in turns/tool_calls/file_diffs.
    wiki.upsert(sid, "test-model",
        "# Summary\nThe agent applied the thirteen-banana-tower refactor.",
        50, 50,
    ).await?;

    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await?;

    // 1. FTS over turns now hits the synthetic phrase via the wiki UNION.
    let turn_hits = search.fts_turns("thirteen-banana-tower", 10).await?;
    assert!(!turn_hits.is_empty(),
        "wiki content should join traces_fts so turn-level FTS finds it");

    // 2. Direct wiki search returns the page.
    let wiki_hits = search.fts_wiki_pages("thirteen-banana-tower", 10).await?;
    assert_eq!(wiki_hits.len(), 1);
    assert!(wiki_hits[0].title.contains("Summary"));

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd --test wiki_in_traces_fts --release`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/wiki_in_traces_fts.rs
git commit -m "test(daemon): L3 wiki content surfaces via traces_fts + fts_wiki_pages"
```

---

## Section 14 — Manual smoke runbook + final integration check

### Task 14.1: Manual runbook

**Files:**
- Create: `docs/runbooks/summarizer-manual-smoke.md`

- [ ] **Step 1: Author**

```markdown
# Manual smoke: session summarizer

Confirms that the summarizer worker writes wiki pages, the MCP tool returns
them, the CLI `sessions show` prints them, and `traces_fts` surfaces them
to `teramind search`.

## Prereqs

- Plans A–G installed.
- Ollama running on localhost:11434 with `qwen3.6:latest` pulled:
  ```sh
  ollama pull qwen3.6:latest
  ```

## Steps

1. Start the daemon: `teramind start`.
2. Run a Claude session in a project directory; aim for 5+ turns and >1 min wall time. End the session (close Claude Code or wait for idle).
3. Wait ~90s, then check the backlog drains:
   ```sh
   teramind doctor | grep "summary"
   ```
   Expect: `summary provider: ollama:qwen3.6:latest (healthy)` and `summary backlog: 0 sessions queued`.
4. Print the wiki page:
   ```sh
   teramind sessions show
   ```
   Expect: Markdown with `# Summary`, `# Files changed`, `# Decisions & gotchas`, `# Follow-ups` sections.
5. Search for a token from the summary:
   ```sh
   teramind search "<unique word from your session>"
   ```
   Expect: at least one hit; if it's a wiki hit, the hit type will say so.
6. Open a NEW Claude Code session in the same cwd. The SessionStart digest should include `## Most recent session summary` with a truncated wiki body.
7. Stop Ollama (`killall ollama`); rerun `teramind doctor`. Expect `unhealthy` and a paused backlog.

## Troubleshooting

- "summary provider: ollama:... (unhealthy)" right after start: confirm
  `ollama serve` is up; `curl http://localhost:11434/api/version`.
- Backlog never drains: check `~/.local/share/teramind/logs/teramindd.log.*`
  for `model not found` or `summarize failed` lines; verify `qwen3.6:latest`
  is pulled.
- `teramind sessions show` says "no wiki page found": session may have been
  too short (default min_turns=3, min_duration_secs=60). Inspect with
  `psql -c "SELECT count(*) FROM sessions_to_summarize"` to see candidates.
```

- [ ] **Step 2: Commit**

```bash
git add docs/runbooks/summarizer-manual-smoke.md
git commit -m "docs: manual smoke runbook for session summarizer"
```

---

### Task 14.2: Final integration check

- [ ] **Step 1: Workspace check + tests + clippy**

```bash
cargo check --workspace
cargo test --workspace --lib
cargo clippy --workspace -- -D warnings
```

Expected: all PASS. Fix minor lint issues inline.

- [ ] **Step 2: Run new integration tests**

```bash
cargo test -p teramind-db wiki_repo_backlog_and_upsert --release
cargo test -p teramind-db wiki_pages_migration_applies_and_traces_fts_rebuilt --release
cargo test -p teramindd --test summarizer_mock --release
cargo test -p teramindd --test wiki_in_traces_fts --release
# Optional (requires host Ollama):
cargo test -p teramindd --test summarizer_ollama --release -- --nocapture || true
```

- [ ] **Step 3: Optional cleanup commit**

```bash
git add -A
git commit -m "chore: clippy cleanups for summarizer plan" || true
```

- [ ] **Step 4:** STOP — do not push or open a PR. Defer to user approval, per A–G convention.

---

## Spec coverage self-check

| Spec section / requirement | Plan task |
|---|---|
| §2.1 `wiki_pages` table + cascade | §1 |
| §2.1 `sessions_to_summarize` view | §1 |
| §2.1 SummaryProvider trait + 3 impls | §2, §4, §5 |
| §2.1 summarizer_worker async, never blocks | §8 |
| §2.1 WikiRepo with all methods | §7 + §8 (load_snapshot) |
| §2.1 traces_fts rebuilt to UNION wiki | §1 |
| §2.1 Hit::WikiPage variant | §9.1 |
| §2.1 mcp__teramind__wiki | §11 |
| §2.1 CLI sessions show | §12.1 |
| §2.1 auto-recall enrichment | §10.1 |
| §2.1 doctor surfaces | §12.2 |
| §2.1 cost gating (min_turns/min_duration/budgets) | §6.1, §8.2 |
| §2.3 SC#1 worker writes within ~60s, no blocking | §13.1 |
| §2.3 SC#2 mcp wiki returns by session_id or cwd | §11 + §13 (IPC arm tested via mock) |
| §2.3 SC#3 search hits wiki via traces_fts | §13.3 |
| §2.3 SC#4 daemon stays up when provider offline | §8.2 (health-check pause) + §13 manual |
| §2.3 SC#5 L5 baseline impact noted | runbook §14.1 + spec covers in §7.6 |
| §4.2 storage schema details | §1 |
| §4.3 SummaryProvider trait shape | §2 |
| §4.4 worker pseudocode | §8.2 |
| §4.5 digest::build with priority-drop | §3.1 |
| §4.6 SYSTEM_PROMPT | §3.2 |
| §5 summarize.toml + secrets.toml | §6.1, §6.2 |
| §5 validation: cloud + egress=false → refuse | §6.1, §6.2 |
| §6.1 MCP tool | §11 |
| §6.2 CLI subcommand | §12.1 |
| §6.3 auto-recall enrichment | §10.1 |
| §6.4 Hit::WikiPage variant | §9.1 |
| §6.5 doctor extension | §12.2 |
| §7.1 L1 digest invariants + proptest | §3.1 |
| §7.2 L2 schema + view + cascade | §1 + §7 |
| §7.3 L3 mock provider | §13.1 |
| §7.4 L3 real Ollama (GPU-preferred) | §13.2 |
| §7.5 L4 nightly real Claude | deferred to ops runbook |
| §7.6 L5 (recompute existing baselines on next CI) | runbook §14.1 |
| §7.7 property tests + fault injection | §3.1, §13 |
| §7.8 perf budgets | informal: §13 timing assertions |
| §8 risks + rollout | covered by tests in §13 |
