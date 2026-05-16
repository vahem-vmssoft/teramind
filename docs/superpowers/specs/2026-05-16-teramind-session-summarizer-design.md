# Teramind Session Summarizer — Design Spec

- **Status:** Approved (brainstorming complete; pending implementation plan)
- **Author:** Vahe Momjyan
- **Date:** 2026-05-16
- **Scope:** Spec #2 of the Teramind product roadmap. Adds LLM-generated wiki pages at session end.

---

## 1. Background and motivation

Teramind Core (Plans A–F) captures every coding-agent session as a structured trace; Plan G layered pgvector for semantic retrieval. What still vanishes when a session ends is the **narrative** — "what did we actually accomplish, what decisions were made, what's left to do." A developer revisiting a project after a week wants a one-page recap, not 200 raw turns.

This spec adds a **session summarizer**: a background worker that builds a structured digest from each ended session, sends it to a pluggable chat LLM (default Ollama on localhost, with opt-in cloud providers), and stores the generated Markdown in a new `wiki_pages` table. The wiki content joins the existing `traces_fts` materialized view so every existing search surface (`teramind search`, `mcp__teramind__search`, slash commands) returns summaries for free. A new MCP tool `mcp__teramind__wiki` and a new CLI subcommand `teramind sessions show <id>` expose the summaries directly. Auto-recall (SessionStart digest) gains a "previous session summary" section.

The summarizer preserves the local-first promise from Core spec §2: default provider is Ollama; cloud providers refuse to construct unless `network_egress = true` in config.

## 2. Goals and non-goals

### 2.1 In scope (v1.0)

- A new `wiki_pages` table keyed by `(session_id, model)` with cascade-delete from `sessions`.
- A `sessions_to_summarize` view + worker filter so the daemon can find ended sessions lacking a summary for the active model.
- A `SummaryProvider` trait + three implementations: `OllamaChatProvider` (default), `AnthropicProvider` (gated on `network_egress=true`), `OpenaiProvider` stub for v1.1.
- A new daemon service `summarizer_worker`: polls, builds a structured digest, applies the existing `Redactor`, calls the active provider, persists Markdown. Async; never blocks ingest or search.
- A `WikiRepo` with `fetch_sessions_to_summarize`, `load_snapshot`, `upsert`, `get_for_session`, `latest_for_project`, `mark_skipped`, `backlog`.
- `traces_fts` materialized view rebuilt to UNION-include `wiki_pages.content` so the existing search blend (FTS + pg_trgm + semantic from Plan G) hits wiki pages.
- A new `Hit::WikiPage` variant in `teramind-core` for direct wiki hits.
- New MCP tool `mcp__teramind__wiki(session_id?, cwd?)`.
- New CLI subcommand `teramind sessions show [<id>] [--json]`.
- Auto-recall (`do_auto_recall`) includes the latest wiki for the cwd's project.
- `teramind doctor` surfaces summarizer provider health + backlog.
- Cost gating via config: `min_turns`, `min_duration_secs`, `input_char_budget`, `output_token_budget`, `max_summary_per_day` (cloud only).

### 2.2 Explicit non-goals (deferred to follow-on revisions)

- Wiki page editing via `$EDITOR` — v1.2.
- Cross-session "project digest" that summarizes the wiki pages themselves — separate spec.
- LLM-generated structured tags / decisions tables — v1.0 outputs Markdown only.
- Wiki page embeddings (semantic search over summaries) — v1.1.
- Streaming output / incremental UI — v1.0 is one-shot per session.
- Map-reduce chunking for very long sessions — YAGNI; the digest's char budget caps input.

### 2.3 Success criteria

1. After `teramind init` on a host with Ollama serving `qwen3.6:latest`, the summarizer worker fills `wiki_pages` rows for newly ended sessions within ~60 s, with zero impact on ingest or search latency.
2. `mcp__teramind__wiki()` (no session_id) returns the most recent summary for the cwd's project. Empty when no sessions have ended in this project yet.
3. `teramind search "<word from a summary>"` returns the corresponding `Hit::WikiPage` plus the relevant session's turns.
4. If Ollama is offline, the daemon stays up, the worker quietly pauses, and `teramind doctor` surfaces the outage. Hooks and search are unaffected.
5. The L5 lexical baseline from Plan F is recomputed once wiki content joins the FTS view; the gates either pass or the PR includes `[eval-baseline-update]`.

## 3. High-level architecture

One new component (`summarizer_worker`) added to the existing daemon layout. No new processes; no IPC-contract breaking changes (one new `Request` variant added).

```
╔════════════════════════════════════════════════════════════════════╗
║                       teramindd                                     ║
║                                                                    ║
║   ingest   fs_watcher   storage_stats   search   embedding_worker   ║
║      │          │             │           │            │            ║
║      ▼          ▼             ▼           ▼            ▼            ║
║   ┌─────────────────────────────────────────────────────────┐      ║
║   │ Postgres pool                                            │     ║
║   │                                                          │     ║
║   │  sessions  turns  tool_calls  file_diffs  embeddings     │     ║
║   │           wiki_pages   ← NEW                            │     ║
║   │           sessions_to_summarize (VIEW) ← NEW            │     ║
║   │           traces_fts (MV, REBUILT to UNION wiki_pages)  │     ║
║   └─────────────────────────────────────────────────────────┘     ║
║                                ▲                                    ║
║                                │ polls + writes                     ║
║   ┌────────────────────────┐   │                                    ║
║   │  summarizer_worker     │───┘   ← NEW                            ║
║   │  (poll → snapshot →    │                                        ║
║   │   digest → redact →    │                                        ║
║   │   provider.summarize → │                                        ║
║   │   upsert wiki_pages)   │                                        ║
║   └───────────┬────────────┘                                        ║
║               │                                                     ║
║               ▼                                                     ║
║   ┌────────────────────────┐                                        ║
║   │  SummaryProvider       │  ← NEW trait, three impls              ║
║   │   OllamaChatProvider   │     (default; /api/chat)               ║
║   │   AnthropicProvider    │     (network_egress=true required)     ║
║   │   OpenaiProvider stub  │     (v1.1)                             ║
║   └────────────────────────┘                                        ║
╚════════════════════════════════════════════════════════════════════╝

                                    │
                                    ▼
                       ┌─────────────────────────┐
                       │ http://localhost:11434  │  (Ollama)
                       └─────────────────────────┘
```

**Layer responsibilities (delta over Core + pgvector):**

- **`wiki_pages` table** — decoupled from `sessions` via `ON DELETE CASCADE`, model-versioned via the unique key. Re-summarization with a different model is sparse-fill friendly.
- **`summarizer_worker`** — single-writer pipeline mirroring `embedding_worker`. Capture-safe: ingest and search never block.
- **`SummaryProvider` trait** — concrete impls under `crates/teramindd/src/services/summarize/`; trait + shared types live in `crates/teramind-core/src/summarize.rs` so the MCP/eval crates can depend on it without pulling in the daemon.
- **`traces_fts` rebuild** — additive: the existing UNION grows a wiki-content branch. Every existing search surface picks up summaries.
- **`Hit::WikiPage`** — a new variant alongside `Hit::Turn`, `Hit::ToolCall`, `Hit::FileDiff`, `Hit::Skill`. Direct hits on wiki content surface as wiki hits; turns whose session has a wiki page still hit through `Hit::Turn` via the UNION.

## 4. Components and storage

### 4.1 Workspace layout (delta)

```
crates/teramind-core/
└── src/
    └── summarize.rs                  ← NEW: SummaryProvider trait + shared types

crates/teramindd/
└── src/services/
    ├── summarize/
    │   ├── mod.rs                    ← NEW: provider factory
    │   ├── ollama.rs                 ← NEW: OllamaChatProvider
    │   ├── anthropic.rs              ← NEW: AnthropicProvider
    │   ├── openai.rs                 ← NEW: stub (v1.1)
    │   ├── digest.rs                 ← NEW: structured digest builder
    │   └── prompts.rs                ← NEW: SYSTEM_PROMPT constant
    └── summarizer_worker.rs          ← NEW: poll/digest/summarize/persist loop

crates/teramind-db/
├── migrations/
│   └── 20260516000002_wiki_pages.sql ← NEW
└── src/repos/
    └── wiki.rs                       ← NEW: WikiRepo

crates/teramind-core/
└── src/types/
    └── hit.rs                        ← MODIFIED: add WikiPage variant

crates/teramind-ipc/
└── src/proto.rs                      ← MODIFIED: Request::WikiLookup / Response::WikiPage

crates/teramind-mcp/
└── src/server.rs                     ← MODIFIED: mcp__teramind__wiki tool

crates/teramind/
└── src/commands/
    └── sessions.rs                   ← MODIFIED: add `show` subcommand
```

### 4.2 Storage: schema

Migration `20260516000002_wiki_pages.sql`:

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

-- Refresh the FTS materialized view so wiki content joins the document.
DROP MATERIALIZED VIEW traces_fts;
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

CREATE INDEX traces_fts_document    ON traces_fts USING gin (document);
CREATE UNIQUE INDEX traces_fts_turn_id ON traces_fts (turn_id);
```

**Key decisions:**

- **Separate `wiki_pages` table** (not a column on `sessions`). Sparse-fill across providers/models; multiple model versions coexist; survives provider swaps.
- **`ON DELETE CASCADE`** — deleting a session removes its wiki. Embeddings (Plan G) used the opposite pattern because re-embedding is expensive; re-summarization is cheap, so cascade is fine.
- **`traces_fts` rebuild** — adds a fifth UNION source. Every existing search surface picks up summaries for free.
- **No HNSW on wiki vectors yet.** Wiki embedding is a v1.1 follow-up; v1.0 ships pure-text search via `traces_fts`.

**Sizing estimate:** Typical summary is ~1500 tokens output ≈ ~6 KB Markdown. 10k sessions × 6 KB ≈ **60 MB**, plus FTS index overhead ≈ ~120 MB. Trivial compared to Core's ~20 GB budget.

### 4.3 `SummaryProvider` trait

```rust
// crates/teramind-core/src/summarize.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
pub use crate::embed::ProviderKind;   // reuse the local/cloud taxonomy

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
```

Three implementations under `crates/teramindd/src/services/summarize/`:

| Provider | Wire | Default model | Setup |
|---|---|---|---|
| `OllamaChatProvider` (**default**) | HTTP POST `http://localhost:11434/api/chat` | `qwen3.6:latest` | User runs `ollama pull qwen3.6` once. Health probe: `GET /api/version`. |
| `AnthropicProvider` | HTTPS POST `api.anthropic.com/v1/messages` | `claude-haiku-4-5-20251001` | Opt-in. Requires `network_egress = true` + `anthropic_api_key` in `~/.config/teramind/secrets.toml`. |
| `OpenaiProvider` | v1.0 stub | — | v1.1 wires it. v1.0 refuses to construct without `network_egress = true`. |

### 4.4 `summarizer_worker` service

```
poll_interval     = 30s
min_turns         = 3
min_duration_secs = 60

loop {
    sleep(poll_interval)
    if !provider.health_check().await.is_ok() { count_error; continue }

    candidates = repo.fetch_sessions_to_summarize(active_model, limit=1).await
    if candidates.is_empty() { continue }

    let s = candidates[0]
    let snapshot = repo.load_snapshot(s.session_id).await
    if snapshot.turn_count < min_turns
       || duration_secs(snapshot) < min_duration_secs {
        repo.mark_skipped(s.session_id, active_model).await
        continue
    }

    let digest = digest::build(&snapshot, input_char_budget)
    let digest = redactor.apply(&digest)        // ALWAYS redact before LLM call

    match provider.summarize(SYSTEM_PROMPT, &digest, output_token_budget).await {
        Ok(result) => repo.upsert(s.session_id, active_model, &result.content,
                                  result.input_tokens, result.output_tokens).await,
        Err(SummaryError::Unhealthy(_)) => continue,
        Err(e) => count_error; warn!(?e),
    }
}
```

**Properties:**

- **Single writer** to `wiki_pages`. `ON CONFLICT (session_id, model) DO NOTHING` for idempotency.
- **Capture-safe.** Hooks, ingest, and search never block on the worker.
- **Outage-resilient.** Provider failures just retry next tick.
- **Daily cap** (cloud only): tracks `max_summary_per_day` from config; the count is persisted in a `summarizer_budget` table (date PK + count) so daemon restarts don't reset.
- **Counters** on `IngestStats`: `summaries_written`, `summaries_errors`, `summaries_skipped`, `summaries_backlog`.

### 4.5 Digest construction (`digest::build`)

Pure function (no I/O, no async), in `crates/teramindd/src/services/summarize/digest.rs`. Deterministic for fixed input. Output is the user-prompt half of the LLM call.

```rust
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

pub fn build(s: &SessionSnapshot, char_budget: usize) -> String;
```

Output (the LLM's user prompt) is structured Markdown with these sections, in order:

```markdown
# Session digest

- session_id: 8f3a…
- cwd: /Users/.../proj
- duration: 42m 11s
- git branch / head: feat/x at abc1234
- ended: stop_hook
- turns: 18    tool calls: 47    files changed: 9

## Tool usage (top 5 by count)
- Edit ×14  (errors: 0)
- Bash ×11  (errors: 2)
- ...

## Files changed
- src/auth/jwt.rs (rust, agent) — 3 hunks (+24, -8)
- ...

## Key prompts (longest user inputs, up to 5)
1. > "..."

## Key assistant outputs (longest, up to 5)
1. > "..."

## Notable tool errors (up to 3)
- Bash (cargo test): "..."

## Diff samples (one per file, top 2 files by total churn)
```rust
// src/auth/jwt.rs
@@ ... @@
- if claims.exp < now() { ... }
+ if claims.exp + clock_skew_secs < now() { ... }
```
```

When the digest exceeds `char_budget`, sections are dropped in this priority order:

1. Drop diff samples (keep file-changes list).
2. Drop "Key assistant outputs" before "Key prompts".
3. Drop "Notable tool errors" last (small but high-signal).
4. Final fallback: truncate the file-changes list to top 10 by total churn.

### 4.6 LLM prompts

Two compile-time constants in `crates/teramindd/src/services/summarize/prompts.rs`. The system prompt is checked into git as a snapshot test (catches accidental drift):

```rust
pub const SYSTEM_PROMPT: &str = r#"
You are summarizing a Claude Code session for a developer wiki. The user
has given you a structured digest of what happened. Write a concise wiki
page in Markdown with these sections, in order:

# Summary

A one-paragraph (~3 sentences) plain-English description of what the
session accomplished, who initiated it (agent vs human edits), and the
outcome.

# Files changed

A bulleted list of files with a one-sentence note per file describing
the intent of the change.

# Decisions & gotchas

3-5 bullets. Surface non-obvious decisions and gotchas the agent noted.
If none are visible in the digest, write "None recorded."

# Follow-ups

Tasks left undone or implied by the work. If none, write "None recorded."

Constraints:
- Be faithful to the digest. Do NOT invent details not present.
- Cite filenames and tool names verbatim where relevant.
- Output Markdown only. No preamble.
"#;
```

## 5. Configuration

One new file `~/.config/teramind/summarize.toml`:

```toml
provider = "ollama"             # ollama | anthropic | openai
model    = "qwen3.6:latest"

poll_interval_secs       = 30
min_turns                = 3     # skip sessions shorter than this
min_duration_secs        = 60    # skip sessions shorter than this
input_char_budget        = 16000
output_token_budget      = 1500
max_summary_per_day      = 100   # only enforced for cloud providers

# Required to use a cloud provider. Refused otherwise.
network_egress = false

[ollama]
url = "http://localhost:11434"
request_timeout_ms = 60000

[anthropic]
# api_key_field = "anthropic_api_key"   # read from secrets.toml
```

**Validation rules:**

- `provider in {anthropic, openai}` + `network_egress = false` → daemon refuses to start with actionable error.
- `secrets.toml` permissions must be `0600`. Daemon refuses to read it if wider; `teramind init` enforces.
- Switching `provider` or `model` does NOT migrate old rows. The view's worker filter (`WHERE NOT EXISTS … WHERE model = $active_model`) naturally re-queues all sessions for the new key. Old rows stay for rollback.
- `max_summary_per_day` ignored for local providers (Ollama, Fastembed-style local).

## 6. MCP / CLI / auto-recall surfaces

### 6.1 MCP tool: `mcp__teramind__wiki`

New tool alongside `search`, `recall`, `save_skill` in `crates/teramind-mcp/src/server.rs`:

```rust
#[tool(description = "Read a session's wiki page. Without session_id, returns the most recent summary for the cwd's project.")]
async fn wiki(
    &self,
    Parameters(p): Parameters<WikiParams>,
) -> Result<CallToolResult, McpError> { /* ... */ }

#[derive(Deserialize, JsonSchema)]
struct WikiParams {
    session_id: Option<String>,
    cwd: Option<String>,
}
```

Result (JSON):

```json
{
  "session_id": "8f3a…",
  "cwd": "/proj/x",
  "model": "ollama:qwen3.6:latest",
  "generated_at": "2026-05-16T14:22:01Z",
  "content": "# Summary\n…"
}
```

Wire shape: a new `Request::WikiLookup { session_id, cwd }` + `Response::WikiPage { … }` in the IPC contract. The CLI subcommand below uses the same IPC.

### 6.2 CLI: `teramind sessions show [<id>]`

Extends the existing `teramind sessions` subcommand from Plan A. If `<id>` is omitted, defaults to the most recent session for the current working directory's project.

```
$ teramind sessions show 8f3a…
# Summary
…

$ teramind sessions show               # implicit: latest for $PWD
# Summary
…

$ teramind sessions show --json 8f3a…  # structured for shell pipelines
{ "session_id": "…", "content": "…", "model": "…", "generated_at": "…" }
```

When the session is found but has no wiki page (still pending / was skipped / summarizer is disabled):

```
teramind: session 8f3a… has no wiki page. Status: pending (backlog=4).
Run `teramind doctor` for summarizer health.
```

### 6.3 Auto-recall enrichment

`do_auto_recall` in `crates/teramindd/src/services/search.rs` (currently merges recent turns + diff excerpts) gains a third source: the latest wiki for the cwd's project.

```rust
let (recent_turns, diff_excerpts, latest_wiki) = tokio::try_join!(
    repo.recent_turns_in_project(None, &req.cwd, req.limit),
    repo.diff_excerpts_for_cwd_files(&req.cwd_files, req.limit),
    wiki_repo.latest_for_project(&req.cwd),
)?;
```

`render_auto_recall_md` opens with a wiki section when one exists, capped at ~1.5 KB (the rest is available via `mcp__teramind__wiki`):

```markdown
## Most recent session summary

> *Generated 2026-05-16 from session 8f3a…*

# Summary
The agent refactored JWT validation to tolerate ±2s clock skew...
```

### 6.4 `Hit::WikiPage` variant

Added in `crates/teramind-core/src/types/hit.rs`:

```rust
pub enum Hit {
    Turn      { … },
    ToolCall  { … },
    FileDiff  { … },
    Skill     { … },
    WikiPage  {
        page_id:    WikiPageId,
        session_id: SessionId,
        title:      String,        // first H1 from content, or "(untitled)"
        snippet:    String,
        score:      f32,
        ts:         OffsetDateTime,
    },
}
```

Direct hits on wiki content surface as `WikiPage`; turns whose session has a wiki page still surface as `Turn` via the FTS UNION (because the wiki content is joined to every turn in the session).

### 6.5 `teramind doctor` extension

Three new lines:

```
summary provider:  ollama:qwen3.6:latest (healthy)
summary backlog:   2 sessions queued
summaries written: 47 total / 0 errors today
```

When the provider is unhealthy:

```
summary provider:  ollama:qwen3.6:latest (unreachable since 2026-05-16T14:22:01Z)
summary backlog:   12 sessions queued (worker paused)
```

## 7. Testing strategy

Five-layer model from Core spec §9.

### 7.1 L1 — Unit (pure logic, no I/O)

- `digest::build` invariants: output length ≤ `char_budget`; deterministic for fixed input.
- Priority-drop ordering: a proptest sweeps `char_budget` from 1024 → 32k and asserts digest length is monotonically non-decreasing.
- Long-prompt truncation: a single 50 KB turn doesn't blow the budget and doesn't split mid-codepoint.
- `SYSTEM_PROMPT` snapshot test (catches accidental edits).
- Config parsing: malformed `summarize.toml` → actionable error; provider + `network_egress` validation matrix.
- `MockSummaryProvider` (deterministic content keyed by input hash) for offline L3 tests.

### 7.2 L2 — Component (per-crate, real embedded Postgres)

- Migration applies; `wiki_pages` table + indexes exist; rebuilt `traces_fts` UNION includes wiki content; `sessions_to_summarize` view returns rows.
- `WikiRepo::fetch_sessions_to_summarize` excludes sessions already summarized for the active model and includes them after a model swap.
- `WikiRepo::upsert` honors `ON CONFLICT (session_id, model) DO NOTHING`.
- `WikiRepo::latest_for_project` returns the most recent wiki for sessions whose cwd matches.
- Cascade-delete a session → its `wiki_pages` rows disappear.

### 7.3 L3 — Integration (full daemon, mock provider)

- Spawn the daemon with `MockSummaryProvider`; ingest a synthetic session (Plan D's `common::Harness`); end via `IngestEvent::SessionEnd`; assert the worker writes a `wiki_pages` row within 60 s.
- Short-session skip: 1 turn + duration < `min_duration_secs` → no wiki row; the sentinel "skipped" mark prevents re-evaluation.
- Provider-swap test: rows with `model="A"`, swap config to `model="B"`, restart, confirm view re-queues all sessions and the worker re-summarizes without touching A-model rows.
- `do_auto_recall` includes the wiki section when present.
- `mcp__teramind__wiki` returns the correct page for an explicit `session_id` and falls back to the latest for the cwd otherwise.
- `traces_fts` hit on wiki content: insert a wiki page with a unique token, search for the token, assert at least one `Hit::WikiPage` plus one `Hit::Turn` joined via session.

### 7.4 L3 — Integration (real Ollama, GPU-preferred)

Provider discovery order (same as Plan G):

1. Probe `http://localhost:11434/api/version`. If responsive AND `qwen3.6:latest` is pulled, use that host-local install. **Uses host GPU automatically** (Metal / CUDA / ROCm).
2. Else: probe `ollama` binary on PATH. If present, `ollama serve` + `ollama pull qwen3.6:latest` in the test setup. Local install still benefits from host GPU.
3. Else (CI runners without Ollama): spawn a managed Docker sidecar with CPU-only image. Significantly slower; tagged separately.

Tests at this layer:

- End-to-end: ingest a real-shape session (10 turns, 3 file diffs); run the worker against a live model; assert the wiki page contains the section headers from `SYSTEM_PROMPT` (`# Summary`, `# Files changed`, `# Decisions & gotchas`, `# Follow-ups`).
- Outage simulation: kill `ollama` mid-summarize; observe worker retry on next tick; confirm no data loss.

### 7.5 L4 — E2E with real Claude Code (nightly)

- Real Claude session producing ~10 turns; wait 90 s after session end; query `mcp__teramind__wiki` from a new session in the same cwd; assert non-empty `content` and at least one filename from the original session appears.

### 7.6 L5 — Search effectiveness benchmark (extension)

- The `traces_fts` rebuild causes wiki content to influence existing lexical scores. The L5 baseline (`baseline.json` from Plan F) is recomputed on the next CI run; merges past the regression gate either pass or use `[eval-baseline-update]`.
- The semantic baseline (`baseline-semantic.json` from Plan G) is similarly affected.
- No new L5 mode is added in v1.0. A `--include-wiki` flag is a v1.1 ergonomic.

### 7.7 Property-based and fault-injection tests

- For any `SessionSnapshot`, `digest::build` output length ≤ `char_budget` (proptest).
- Sweeping `char_budget` from 1024 → 32k yields monotonically non-decreasing digest length.
- For any `MockSummaryProvider` output, `wiki_pages.content` stored matches what the provider returned (no extra processing in the worker).
- Kill Ollama mid-summarize → worker logs error, retries next tick, no data lost.
- Provider returns `model not found` → worker pauses; `teramind doctor` shows actionable error.
- Daily cap (cloud) reached → worker emits warning, defers remaining sessions to next UTC day.
- Wiki page exceeds 100 KB (pathological output) → DB write succeeds; FTS handles it.

### 7.8 Performance budgets

| Path | Budget |
|---|---|
| `digest::build` (10k-turn session, 32k char budget) | p99 < 50 ms |
| Worker per-session wall time (Ollama qwen3.6 on M-series Mac) | p99 < 60 s for ≤ 32k input chars |
| Backlog drain rate (single worker, single concurrent call) | ≥ 1 session/minute |
| `traces_fts` refresh after wiki insert (CONCURRENTLY, 10k sessions) | p99 < 30 s |

## 8. Rollout, dependencies, risks

### 8.1 Dependencies

- Spec depends on Teramind Core (Plans A–F) and pgvector (Plan G — for the `ProviderKind` type and the local/cloud taxonomy).
- Does NOT depend on the other follow-on specs (skill codifier, team sync).
- Skill codifier (spec follow-on #3) can use wiki pages as a candidate source for "what patterns repeat across sessions" — sequence pgvector → summarizer → codifier is the natural ordering.

### 8.2 Rollout phases

1. **v1.0 (this spec)** — schema, worker, Ollama default + Anthropic stub, MCP + CLI + auto-recall + doctor, fail-soft L5 instrumentation.
2. **v1.0.1** — `baseline.json` / `baseline-semantic.json` recomputed after wiki content is exercised at scale. Tune FTS weights if wiki text dominates.
3. **v1.1** — Anthropic + OpenAI cloud providers fully wired; wiki page embeddings (semantic search over summaries) layered on Plan G's pgvector.
4. **v1.2** — User-edited wiki via `teramind sessions edit <id>`; "locked" flag prevents regeneration of manually-edited pages.

### 8.3 Open questions resolved during plan execution

- **Daily-cap accounting.** Persist counter in a `summarizer_budget` table keyed on UTC date so daemon restarts don't reset.
- **`gen_random_uuid()` availability.** `pgcrypto` was enabled in Plan A; verify the migration still applies cleanly on the embedded PG bundle.
- **Anthropic API version pin.** Use `anthropic-version: 2023-06-01` for v1.0; revisit on v1.1 if API surfaces shift.
- **Backfill on first install.** Existing sessions in the DB at install time are eligible immediately — the view doesn't care. The worker drains them at `1/poll_interval` rate (default ~120/hour).

### 8.4 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Long sessions exceed Ollama context window despite digest budget | Medium | Char-capped digest (16k default, ~4k tokens). Priority drop ordering keeps high-signal sections. v1.1 chunking is optional. |
| Hallucination in summaries | Medium | Prompt forbids inventing details; digest is structured factual. L4 nightly assertion ("a filename from the original session appears") is a weak hallucination detector. |
| Slow Ollama on consumer hardware | Medium | `request_timeout_ms` defaults to 60s. Worker is async — slow summaries don't block anything else. Backlog metric tells users when they're falling behind. |
| Local model unavailable on first run | High | Daemon stays up; worker logs that the model isn't pulled; `teramind doctor` surfaces actionable error: "Run `ollama pull qwen3.6` to enable." |
| Wiki content pollutes `traces_fts` enough to skew lexical search | Low | Wiki joins to every turn in the session, so matching wiki text ranks every turn similarly. Tune via `setweight()` in the materialized view if it becomes a problem. |
| Privacy: summaries land in DB even for sensitive sessions | Medium | Same `Redactor` runs on the digest before LLM call. Cloud-provider users get redacted input sent to the vendor. Documented prominently. |
| Daily-cap drift across daemon restarts | Low | Counter persisted in `summarizer_budget` table (date PK + count). |

### 8.5 Out of scope (deferred to later revisions / follow-on specs)

- Wiki page editing — v1.2.
- Cross-session "project digest" — separate spec.
- LLM-generated tags / decisions tables — outputs Markdown only.
- Wiki page embeddings — v1.1.
- Streaming output — v1.0 is one-shot.
- Map-reduce chunking — YAGNI; the char budget caps input.

## 9. Glossary

- **Wiki page** — the Markdown summary stored in `wiki_pages.content`. One per `(session_id, model)`.
- **Summary provider** — a `SummaryProvider` impl (Ollama / Anthropic / OpenAI / etc.).
- **Digest** — the structured Markdown the daemon hands to the LLM as the user prompt. Deterministically constructed by `digest::build` from a `SessionSnapshot`. Capped at `input_char_budget` chars.
- **Backlog** — count of ended sessions in `sessions_to_summarize` that lack a wiki page for the active model. Surfaced via `teramind doctor`.
- **Sentinel skip** — a `wiki_pages` row with empty content used to mark sessions deliberately skipped (too short, etc.) so the worker doesn't keep re-evaluating them.
- **Provider swap** — changing `summarize.toml.provider` or `summarize.toml.model`. Triggers re-summarization without touching old rows; rollback is "switch back and the old rows are still there."
- **Network egress** — outbound HTTPS to a non-localhost host. Refused unless the user opts in via `network_egress = true`.
