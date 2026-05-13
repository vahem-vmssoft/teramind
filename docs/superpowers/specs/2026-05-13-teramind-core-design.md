# Teramind Core — Design Spec

- **Status:** Approved (brainstorming complete; pending implementation plan)
- **Author:** Vahe Momjyan
- **Date:** 2026-05-13
- **Scope:** Spec #1 of the Teramind product roadmap. Single-user, single-machine substrate.

---

## 1. Background and motivation

Engineers across the org use AI coding agents (Claude Code today; Codex, Cursor, Hermes, Pi in future) in silos. Every session — its prompts, the model's reasoning, the tool calls, the file changes that resulted — vanishes when the session ends. Solved bugs are re-solved, productive patterns are re-invented, and an entire class of organizational learning never accumulates.

**Teramind** is the substrate that captures every coding-agent interaction in the org as a structured trace, lets every agent search prior traces, codifies repeated patterns into reusable skills, and (eventually) propagates capability across teammates and machines in real time.

This spec covers **Teramind Core**: the single-machine, single-user foundation. Team sync, the LLM-driven session summarizer, the pattern → skill codifier, and additional agent connectors each get their own follow-on spec. They all sit on top of what is specified here.

## 2. Goals and non-goals

### 2.1 In scope (v1)

- A Rust workspace producing four binaries: `teramindd` (daemon), `teramind` (CLI), `teramind-hook` (event shim), `teramind-mcp` (MCP server).
- An embedded Postgres instance managed as a child process of the daemon, with `pg_trgm` and `pgcrypto` extensions enabled.
- A Claude Code plugin bundle, installed via `teramind claude install`, containing hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `PreCompact`), an MCP server entry, and `/teramind:search` / `/teramind:recall` slash commands.
- **Full-fidelity trace capture:** every prompt, assistant message, tool call (name + input + output), thinking block, and model metadata (model, tokens, timestamps, cwd, git HEAD).
- A filesystem watcher that captures **per-turn diffs keyed by content excerpt**, with attribution to either the responsible agent turn or to the human user.
- Four search/recall surfaces: `teramind search` (CLI), MCP tools (`search`, `recall`, `save_skill`), Claude slash commands, and an auto-recall digest injected via the `SessionStart` hook.
- Cross-platform installer for macOS (notarized), Linux (unsigned), and Windows (unsigned in v1, signed in v1.1).
- An agent-agnostic internal schema so that adding Codex / Cursor / Hermes / Pi connectors in later specs does not require schema migrations.
- A search effectiveness evaluation harness (L5 test layer) with a labelled query/relevance corpus and CI regression gates.

### 2.2 Explicit non-goals (deferred to follow-on specs)

- The AI session summarizer that produces wiki pages at session end.
- The pattern → skill codifier that mines repeated patterns across traces.
- Team sync server, authentication, multi-machine replication.
- Connectors for Codex, Cursor, Hermes, Pi (schema must accommodate but no connector ships).
- Any web UI, dashboard, or hosted offering.
- `pgvector` / embedding-based semantic search.
- Auto-registration as an OS service (launchd / systemd / Windows Service).

### 2.3 Success criteria

1. A fresh user can run a single install script on macOS, Linux, or Windows, then `teramind init && teramind claude install`, open Claude Code, run a session, and find every prompt, assistant message, tool call, and file change persisted to local Postgres within Teramind's stated latency budgets.
2. `teramind search "<query>"` returns ranked results across all prior local sessions; p95 latency ≤ 5 s on a 10k-session corpus, target ≤ 800 ms.
3. From inside Claude Code, the model can call `mcp__teramind__recall` and receive structured prior-context hits.
4. `teramind claude uninstall` cleanly removes the plugin without touching user data; `teramind uninstall --purge` removes everything.
5. The L5 search-effectiveness benchmark establishes a baseline on `main` (nDCG@10, MRR, P@5, P@10, R@10 across five query classes) that PRs must not regress past the documented thresholds.

## 3. High-level architecture

Three layers: a declarative Claude-side plugin, a set of local runtime processes (one stateful daemon plus three stateless clients), and the embedded Postgres data store.

```
╔════════════════════════════════════════════════════════════════════╗
║                       CLAUDE-SIDE (per user)                        ║
║  ~/.claude/plugins/teramind/                                        ║
║   ├── plugin.json                                                  ║
║   ├── hooks/         ── thin shell wrappers that exec teramind-hook ║
║   │   ├── session_start.sh                                         ║
║   │   ├── user_prompt_submit.sh                                    ║
║   │   ├── pre_tool_use.sh                                          ║
║   │   ├── post_tool_use.sh                                         ║
║   │   ├── stop.sh                                                  ║
║   │   └── pre_compact.sh                                           ║
║   ├── commands/      ── /teramind:search, /teramind:recall          ║
║   ├── skills/        ── empty in v1; populated by future codifier   ║
║   └── .mcp.json      ── declares teramind-mcp as an MCP server      ║
╚══════════════╤════════════════════════════════════════════════════╝
               │ exec / stdio
               ▼
╔════════════════════════════════════════════════════════════════════╗
║                       LOCAL RUNTIME (per machine)                   ║
║   ┌────────────────┐   ┌──────────────────┐   ┌────────────────┐   ║
║   │ teramind-hook  │   │  teramind-mcp    │   │   teramind     │   ║
║   │  (~5KB shim)   │   │  (stdio MCP)     │   │   (CLI)        │   ║
║   └────────┬───────┘   └────────┬─────────┘   └────────┬───────┘   ║
║            │ JSON-RPC over Unix Domain Socket (or Named Pipe on Win)║
║            └────────────────────┼────────────────────────┘         ║
║                                 ▼                                  ║
║   ┌────────────────────────────────────────────────────────────┐   ║
║   │                       teramindd                            │   ║
║   │  ┌────────────┐ ┌──────────────┐ ┌─────────────────────┐   │   ║
║   │  │ ingest svc │ │ search svc   │ │ session manager     │   │   ║
║   │  └─────┬──────┘ └──────┬───────┘ └──────────┬──────────┘   │   ║
║   │  ┌─────▼───────────────▼─────┐  ┌───────────▼──────────┐   │   ║
║   │  │ pg connection pool        │  │  fs watcher (notify) │   │   ║
║   │  └─────┬─────────────────────┘  └───────────┬──────────┘   │   ║
║   │  ┌─────▼──────────────────────────────────────────────┐    │   ║
║   │  │ embedded postgres supervisor (child process)        │   │   ║
║   │  └────────────────────────┬──────────────────────────┘    │   ║
║   └─────────────────────────────┼────────────────────────────┘    ║
╚═══════════════════════════════════╪════════════════════════════════╝
                                    ▼
                ╔══════════════════════════════════════╗
                ║   Postgres (UDS preferred,            ║
                ║   localhost:5436 fallback)           ║
                ║   db: teramind                       ║
                ║   ext: pg_trgm, pgcrypto             ║
                ╚══════════════════════════════════════╝
```

**Layer responsibilities.**

- **Claude plugin** — declarative; routes Claude events into `teramind-hook` and exposes tools/commands. Owns no state.
- **Runtime clients** — `teramind-hook`, `teramind-mcp`, and `teramind` are stateless. They speak a single JSON-RPC protocol to the daemon over a Unix Domain Socket (`/tmp/teramind.sock`) or Named Pipe (`\\.\pipe\teramind`).
- **`teramindd`** — the only stateful process. Owns the embedded Postgres lifecycle, the FS watcher, the session manager, and the ingest pipeline. Never caches anything not recoverable from Postgres on restart.
- **Postgres** — the source of truth. The daemon's JSONL shadow log (Section 4.4) is a redundant durable buffer, not a primary store.

**Daemon lifecycle.** Lazy-spawned: the first hook invocation tries to connect to the IPC socket within a 50 ms deadline; if it can't, it `exec`s `teramindd --background --detached` and re-tries once after 250 ms. `teramind start | stop | status | restart` are explicit user-facing controls. There is no OS-service auto-registration in v1.

**Cross-platform IPC.** `tokio::net::UnixListener` on Unix, `tokio::net::windows::named_pipe::NamedPipeServer` on Windows. One JSON-RPC contract on both, behind a `cfg`-gated transport adapter in `teramind-ipc`.

## 4. Components and storage

### 4.1 Workspace layout

```
teramind/
├── Cargo.toml                         ── [workspace] members = [...]
├── crates/
│   ├── teramind-core/                 ── lib: shared types, schema, errors, redaction
│   ├── teramind-ipc/                  ── lib: JSON-RPC contract + client/server transport
│   ├── teramind-db/                   ── lib: sqlx queries, migrations, embedded PG supervisor
│   ├── teramindd/                     ── bin: the daemon
│   ├── teramind/                      ── bin: the user-facing CLI
│   ├── teramind-hook/                 ── bin: tiny hook shim
│   └── teramind-mcp/                  ── bin: MCP stdio server (uses rmcp)
├── plugins/
│   └── claude/                        ── Claude Code plugin template
│       ├── plugin.json
│       ├── hooks/*.sh
│       └── commands/*.md
├── migrations/                        ── sqlx-format .sql migrations
├── benches/
│   └── search-eval/                   ── L5 effectiveness corpus + harness
└── installer/                         ── install.sh, install.ps1, release packaging
```

### 4.2 Crate responsibilities

| Crate | Type | Responsibility | Key deps |
|---|---|---|---|
| `teramind-core` | lib | Domain types (`Session`, `Turn`, `ToolCall`, `FileDiff`, `Skill`, `Agent`), serde models, error enum, redaction rules. | serde, thiserror, uuid, time |
| `teramind-ipc` | lib | JSON-RPC contract enum, codecs over UDS/Named Pipe, `IpcClient` trait. | tokio, serde_json, interprocess |
| `teramind-db` | lib | sqlx queries, migration runner, embedded Postgres supervisor (`postgresql_embedded`), pool. Exposes high-level repositories (`SessionRepo`, `TraceRepo`, `DiffRepo`, `SkillRepo`, `SearchRepo`). | sqlx, postgresql_embedded, tokio |
| `teramindd` | bin | Wires services: ingest pipeline, search service, session manager, FS watcher. Hosts the IPC server. Long-running. | tokio, notify, teramind-* libs |
| `teramind` | bin | User CLI. Subcommands: `init`, `start`/`stop`/`status`/`restart`/`doctor`, `claude install`/`uninstall`, `search`, `recall`, `sessions`, `reset`, `uninstall [--purge]`, `self update`, `version`. | clap, teramind-ipc |
| `teramind-hook` | bin | Sub-millisecond cold-start shim. Reads Claude hook JSON from stdin, builds an `IngestEvent`, fires fire-and-forget to the daemon, exits 0. Spawns daemon if absent. | teramind-ipc (subset) |
| `teramind-mcp` | bin | MCP stdio server (built on `rmcp`) implementing tools `search`, `recall`, `save_skill`. Translates MCP calls into IPC against the daemon. | rmcp, teramind-ipc |

### 4.3 Daemon services

- **`ingest`** — single-writer pipeline. Accepts `IngestEvent` from IPC, validates, **applies redaction before any persistence**, persists to Postgres in per-turn transactions. Backpressure via a bounded `tokio::mpsc` channel: when full, callers receive `IpcError::Busy` and the event is dropped after counter increment. Capture is best-effort by design — Teramind never blocks Claude.
- **`session_manager`** — tracks active sessions: `session_id → { cwd, agent, started_at, last_turn_id, pid }`. Driven by `SessionStart` and `Stop` hook events. Used by the FS watcher for turn attribution and by ingest to enforce per-session invariants.
- **`fs_watcher`** — one `notify::RecommendedWatcher` per active session's cwd. Per-file debounce of 200 ms. On change: read post-content, look up pre-content from in-memory snapshot cache (taken on first observed write within a turn) or re-read from the git index when available; compute unified diff; extract pre/post excerpts as ±50 lines around each hunk; attribute to active turn if within 5 s of a `PostToolUse` for `Edit|Write|MultiEdit|NotebookEdit`, else `attribution='human'`.
- **`search`** — Postgres-backed (Section 6) with a grep-fallback path that activates when `SELECT 1` fails twice in a row.
- **`pg_supervisor`** — owns the embedded Postgres `Child`. Starts on daemon launch with per-user data dir `~/.local/share/teramind/pgdata/`, runs migrations to head, restarts on crash with exponential backoff (1 s → 2 s → 4 s → max 60 s), graceful shutdown on daemon `SIGTERM`.

### 4.4 Storage: schema

All UUIDs use `pgcrypto::gen_random_uuid()`.

```sql
-- Agents: connector identity. Claude Code is the only kind wired in v1;
-- the column exists so Codex/Cursor/Hermes/Pi land without migration.
CREATE TABLE agents (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind         text NOT NULL,              -- 'claude_code', 'codex', 'cursor', ...
  version      text,
  installed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (kind, version)
);

-- Projects: stable identity for a working directory.
CREATE TABLE projects (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  root_path    text NOT NULL UNIQUE,
  git_remote   text,
  display_name text,
  first_seen   timestamptz NOT NULL DEFAULT now()
);

-- Sessions: one row per agent invocation.
CREATE TABLE sessions (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  agent_id     uuid NOT NULL REFERENCES agents(id),
  agent_session_id text,
  cwd          text NOT NULL,
  project_id   uuid REFERENCES projects(id),
  parent_session_id uuid REFERENCES sessions(id),  -- sub-agent / Task tool spawns
  git_head     text,
  git_branch   text,
  os           text NOT NULL,
  hostname     text NOT NULL,
  user_login   text NOT NULL,
  started_at   timestamptz NOT NULL,
  ended_at     timestamptz,
  end_reason   text,                       -- 'stop_hook' | 'idle_timeout' | 'crash' | 'compact'
  metadata     jsonb NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX sessions_cwd_started ON sessions (cwd, started_at DESC);
CREATE INDEX sessions_project ON sessions (project_id, started_at DESC);

-- Turns: one row per logical exchange (user prompt -> assistant response).
CREATE TABLE turns (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id   uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  ordinal      integer NOT NULL,
  started_at   timestamptz NOT NULL,
  ended_at     timestamptz,
  user_prompt  text,
  assistant_text text,
  thinking     text,
  model        text,
  input_tokens  integer,
  output_tokens integer,
  UNIQUE (session_id, ordinal)
);
CREATE INDEX turns_session ON turns (session_id, ordinal);

-- Tool calls: one row per tool invocation within a turn.
CREATE TABLE tool_calls (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  turn_id      uuid NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  ordinal      integer NOT NULL,
  name         text NOT NULL,
  input        jsonb NOT NULL,
  output       text,
  is_error     boolean NOT NULL DEFAULT false,
  started_at   timestamptz NOT NULL,
  duration_ms  integer,
  UNIQUE (turn_id, ordinal)
);
CREATE INDEX tool_calls_turn ON tool_calls (turn_id, ordinal);
CREATE INDEX tool_calls_name ON tool_calls (name);

-- File diffs: per-turn FS changes captured by the watcher.
CREATE TABLE file_diffs (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  turn_id      uuid REFERENCES turns(id) ON DELETE CASCADE,
  session_id   uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  file_path    text NOT NULL,              -- absolute
  rel_path     text NOT NULL,              -- relative to session cwd
  attribution  text NOT NULL CHECK (attribution IN ('agent', 'human')),
  language     text,
  pre_excerpt  text NOT NULL,
  post_excerpt text NOT NULL,
  unified_diff text NOT NULL,
  pre_hash     bytea NOT NULL,             -- sha256 of full pre-content
  post_hash    bytea NOT NULL,
  byte_size    integer NOT NULL,
  captured_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX file_diffs_session ON file_diffs (session_id, captured_at DESC);
CREATE INDEX file_diffs_relpath ON file_diffs (rel_path);
CREATE INDEX file_diffs_pre_excerpt_trgm ON file_diffs USING gin (pre_excerpt gin_trgm_ops);
CREATE INDEX file_diffs_post_excerpt_trgm ON file_diffs USING gin (post_excerpt gin_trgm_ops);

-- Skills: reusable skill files. Empty in v1 (no codifier yet); populated via save_skill MCP tool.
CREATE TABLE skills (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name         text NOT NULL UNIQUE,       -- kebab-case slug
  description  text NOT NULL,
  body         text NOT NULL,
  source       text NOT NULL CHECK (source IN ('authored', 'codified', 'imported')),
  source_session_ids uuid[] NOT NULL DEFAULT '{}',
  created_at   timestamptz NOT NULL DEFAULT now(),
  updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX skills_name_trgm ON skills USING gin (name gin_trgm_ops);
CREATE INDEX skills_body_trgm ON skills USING gin (body gin_trgm_ops);

-- Storage stats: disk-usage telemetry, sampled every 5 minutes by the daemon.
CREATE TABLE storage_stats (
  id           bigserial PRIMARY KEY,
  sampled_at   timestamptz NOT NULL DEFAULT now(),
  pg_bytes     bigint NOT NULL,            -- pg_database_size('teramind')
  jsonl_bytes  bigint NOT NULL,            -- du on raw/ dir
  session_count bigint NOT NULL,
  turn_count   bigint NOT NULL,
  diff_count   bigint NOT NULL
);
CREATE INDEX storage_stats_sampled ON storage_stats (sampled_at DESC);
```

**Full-text search:** one materialized view `traces_fts` over `turns`, `tool_calls`, and `file_diffs`, with a `tsvector` column derived from
`coalesce(user_prompt,'') || ' ' || coalesce(assistant_text,'') || ' ' || coalesce(tool_calls.output,'') || ' ' || coalesce(unified_diff,'')`.
Refreshed `CONCURRENTLY` every 30 s by a daemon background task.

**JSONL shadow log:** every ingest event is appended (before Postgres write) to a daily-rotated JSONL at `~/.local/share/teramind/raw/YYYY-MM-DD.jsonl`. This is the grep-fallback substrate, a redundant durable buffer, and the re-import path if Postgres ever becomes unrecoverable.

**Storage telemetry:** the daemon samples `pg_database_size('teramind')` and the JSONL directory size every 5 minutes and inserts a `storage_stats` row. `teramind status` surfaces the latest sample.

**Sizing estimate:** a typical session is ~50 turns × ~5 tool calls × ~2 KB output (~500 KB structured) plus ~10-20 file diffs at ~2 KB each (~50 KB). Round to **~1 MB in Postgres + ~1 MB JSONL per session**. 10k sessions ≈ 20 GB total.

## 5. Capture flow (data flow)

Single Claude turn, end-to-end:

```
T+0 ms     User submits prompt in Claude Code
T+1 ms     UserPromptSubmit hook -> teramind-hook reads stdin, builds
           IngestEvent::UserPrompt { session_id, turn_ordinal, prompt,
           cwd, ts, client_event_id }, fires JSON-RPC notify to daemon,
           exits.
T+4 ms     teramindd.ingest:
              session_manager.upsert(session_id, cwd, agent='claude_code')
              INSERT INTO turns (...) RETURNING id
              APPEND raw JSONL line
              broadcast `turn_started` to fs_watcher
T+5 ms+    fs_watcher registers a watch on cwd (if new) and notes the
           active turn_id with a snapshot-on-first-write policy.

         (Per tool call within the turn:)
PreToolUse  -> IngestEvent::ToolCallStart { turn_id, name, input, ts }
            -> daemon INSERTs tool_calls with output=NULL
              (tool runs; e.g. Edit modifies foo.rs)
PostToolUse -> IngestEvent::ToolCallEnd { tool_call_id, output, is_error,
                                          duration_ms, ts }
            -> daemon UPDATEs tool_calls and broadcasts
               `write_tool_completed` for Edit|Write|MultiEdit|NotebookEdit

         (FS watcher diff capture per modified file, 200 ms debounce:)
            1. Read post-content of the dirty file
            2. Resolve pre-content from in-memory snapshot or git index
            3. Compute unified diff (similar to `git diff --no-index`)
            4. Extract ±50-line excerpts around each hunk
            5. Attribution: 'agent' if within 5 s of a write-tool
               PostToolUse; else 'human'
            6. IngestEvent::FileDiff -> INSERT INTO file_diffs

         (At assistant turn end:)
Stop / PreCompact carry the transcript segment for the turn:
            -> IngestEvent::AssistantTurn { turn_id, assistant_text,
                 thinking, model, input_tokens, output_tokens, ts }
            -> daemon UPDATEs the turns row, finalizes ended_at.

         (At session end:)
Stop hook with stop_hook_active=false -> sessions.ended_at = now(),
                                         end_reason = 'stop_hook'.
Idle sweeper sets end_reason='idle_timeout' after 30 min of silence.
PreCompact is recorded as metadata, NOT a session end.

         (At session open:)
SessionStart hook -> daemon.auto_recall(cwd) -> markdown digest -> stdout
            -> Claude Code injects it into the model's context.
```

**Ordering and idempotency.** Each `IngestEvent` carries a `client_event_id` (UUID minted by `teramind-hook`); the daemon dedupes on it. `turn_ordinal` is assigned client-side from Claude's transcript position. On collision the daemon uses `INSERT ... ON CONFLICT (session_id, ordinal) DO NOTHING` and logs a warning. **Sub-agent turns** (Task / Agent tool spawns) become their own `sessions` row with `parent_session_id` set to the parent.

**Failure behavior (capture-side).**

| Failure | Behavior | Counter |
|---|---|---|
| Hook can't reach daemon | Write event to `~/.local/share/teramind/inbox/<uuid>.json`, exit 0. Daemon drains inbox on startup. | `hook_inbox_writes_total` |
| Daemon ingest channel full | Drop event, increment counter, exit 0. JSONL append still happens via a separate writer. | `ingest_drops_total` |
| Postgres write fails (constraint / transient) | Retry 3× with backoff, then move event to `~/.local/share/teramind/dead_letter/`. | `pg_write_failures_total`, `dead_letter_events_total` |
| Postgres connection refused | Buffer up to 10k events in memory; once full, drop with counter. JSONL keeps appending. Search degrades to grep fallback. | `pg_down_seconds_total`, `search_degraded_total` |
| Embedded Postgres crash | Supervisor restarts with exp backoff (1 s → 2 s → 4 s → max 60 s). After 5 failures, daemon logs FATAL but keeps accepting IPC. | `pg_restarts_total` |
| FS watcher loses inotify slot | Log warning, retry registration after 5 s. Record the outage interval as a structured entry in `audit.log` and append a `{"gap": {...}}` element to the active session's `metadata` JSONB. | `fs_watcher_gaps_total` |
| Migration fails | Daemon refuses to start, prints failed migration ID, DB untouched. Diagnostic via `teramind db migrate --status`. | n/a |
| Disk full | Daemon stops accepting writes, surfaces `degraded: full_disk` on every IPC call, raises log severity. | `disk_full_blocks_total` |
| `teramind-hook` crashes | No-op for Claude. Stack trace to `~/.local/share/teramind/logs/hook.log`. | n/a |

**Contract:** capture is best-effort. Teramind **never blocks Claude.** Every degradation mode is named and counted.

**Redaction (mandatory, applied in `ingest` before any persistence).**

- Default rules (in `teramind-core::redact`): AWS access keys, AWS secret keys, GitHub PAT/OAuth tokens (`ghp_`, `gho_`, `ghs_`, `ghr_`), Slack tokens (`xox[bpoa]-`), generic JWTs, PEM private key blocks, `password=...` / `pwd=...` patterns, and `.env`-style `key=value` where key matches an allowlist (PASSWORD, SECRET, TOKEN, KEY, CREDENTIAL).
- Match replacement: the matched substring becomes `«redacted:aws_access_key»` etc. The original is **never** written.
- User config: `~/.config/teramind/redact.toml` adds project-specific patterns. `teramind redact test <input>` lets users sanity-check rules.
- Off-switch: `teramind init --no-redact` writes a config flag; daemon refuses to read it without `--i-understand-the-risk`.

## 6. Search and retrieval

Four surfaces, one shared search service inside the daemon.

```
┌──────────────────────────────────────────────────────────────────────┐
│  Surfaces (presentation only)                                        │
│    1. CLI:        teramind search "<query>"                          │
│    2. MCP tools:  mcp__teramind__search / __recall / __save_skill    │
│    3. Slash:      /teramind:search · /teramind:recall                │
│    4. Auto:       SessionStart hook -> daemon.auto_recall(cwd)       │
│  All four send a SearchRequest over IPC; the daemon owns ranking.    │
└─────────────────────────────────┬────────────────────────────────────┘
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│  search service pipeline:                                            │
│    1. parse & normalize                                              │
│    2. plan: which indexes to hit                                     │
│         - tsvector (traces_fts) for word/phrase queries              │
│         - pg_trgm GIN on file_diffs.pre_excerpt for code snippets    │
│         - exact-match on tool_calls.name for tool-typed queries      │
│    3. execute concurrent SELECTs                                     │
│    4. rank: blended score (Section 6.1) + recency decay + project    │
│    5. dedupe, hydrate (load surrounding turn for each hit)           │
│    6. return SearchResults { hits[], degraded: bool, took_ms }       │
└──────────────────────────────────────────────────────────────────────┘
```

### 6.1 Ranking

Lexical-only in v1. No embeddings; `pgvector` is a later spec. Two strategies blended:

- **tsvector full-text** via `ts_rank_cd` for natural-language queries.
- **pg_trgm similarity** for code-shaped queries (threshold ~0.3).

```
score = 0.6 * fts_score
      + 0.4 * trgm_score
      + 0.2 * recency_decay     -- exp(-age_days / 90)
      + 0.3 * same_project_boost
```

Tunable via `~/.config/teramind/search.toml`. Tuning changes are gated by the L5 effectiveness benchmark.

### 6.2 Result type

```rust
enum Hit {
    Turn      { turn_id, session_id, ordinal, snippet, score, ts },
    ToolCall  { tool_call_id, turn_id, name, input_snippet, output_snippet, score, ts },
    FileDiff  { diff_id, rel_path, hunk_snippet, score, ts },
    Skill     { skill_id, name, body_snippet, score },
}
```

### 6.3 MCP tool contract

```
mcp__teramind__search(query: string, limit?: int = 10) -> Hit[]
mcp__teramind__recall(
    cwd?: string,
    file_paths?: string[],
    symbols?: string[],
    stack_traces?: string[],
    limit?: int = 10
) -> Hit[]
mcp__teramind__save_skill(name: string, description: string, body: string) -> SkillRef
```

`save_skill` ships in v1 even though the auto-codifier does not, giving users and the model a manual on-ramp. It writes a `skills` row with `source='authored'` and a corresponding `~/.claude/plugins/teramind/skills/<name>/SKILL.md` so Claude picks it up on the next session.

### 6.4 Auto-recall (SessionStart hook)

On Claude session open in a cwd, the daemon runs three queries in parallel:

1. Most-recent 5 turns in the same `project_id` if non-null, falling back to `cwd` match when `project_id` is null.
2. Most-similar 5 `file_diffs` whose `rel_path` matches files currently present in cwd.
3. Top 3 skills matching the project's file extensions.

The merge is rendered as a markdown digest, capped at ~4 KB, printed to stdout, and consumed by Claude's hook contract for context injection. Configurable via `teramind config set autorecall.enabled false`.

### 6.5 Grep fallback

Triggered when `SELECT 1` against Postgres fails twice in a row, or when the daemon starts before Postgres is ready.

- Implementation: `tokio::process::Command` running `grep -rIEn --include='*.jsonl'` over `~/.local/share/teramind/raw/`, with the query escaped as a regex.
- Returns the same `Hit` enum (synthesized from JSONL fields), with `degraded: true`. No ranking — substring match plus line-context. Slower (~1-3 s on a 10k-session corpus).
- CLI flag `teramind search --grep` forces this path for debugging.

## 7. Installation, packaging, lifecycle

### 7.1 Distribution artifacts

CI produces, per tagged release:

```
teramind-<version>-aarch64-apple-darwin.tar.gz      (notarized)
teramind-<version>-x86_64-apple-darwin.tar.gz       (notarized)
teramind-<version>-x86_64-unknown-linux-gnu.tar.gz
teramind-<version>-aarch64-unknown-linux-gnu.tar.gz
teramind-<version>-x86_64-pc-windows-msvc.zip       (unsigned in v1)
teramind-<version>-aarch64-pc-windows-msvc.zip      (unsigned in v1)
teramind-<version>-SHA256SUMS
teramind-<version>-SHA256SUMS.sig                   (cosign)
install.sh
install.ps1
```

Each archive contains: `teramind`, `teramindd`, `teramind-hook`, `teramind-mcp`, the `plugins/claude/` template, and a `LICENSE`. **Embedded Postgres is downloaded on first run** (~50 MB cached after) via the `postgresql_embedded` crate, with `teramind init --offline --postgres-path=<dir>` as an air-gapped escape hatch.

### 7.2 Installer behavior

```bash
# macOS / Linux
curl -fsSL https://get.teramind.dev/install.sh | sh
# Windows (PowerShell)
irm https://get.teramind.dev/install.ps1 | iex
```

Steps:

1. Detect OS + arch.
2. Download the right archive, `SHA256SUMS`, `.sig`. Verify checksum (and signature if `cosign` is on PATH).
3. Extract to `~/.local/share/teramind/bin/` (Unix) or `%LOCALAPPDATA%\teramind\bin\` (Windows).
4. Symlink `teramind` into `~/.local/bin/` (Unix) or prepend `%LOCALAPPDATA%\teramind\bin` to user PATH via `setx` (Windows).
5. Print: `teramind init && teramind claude install`.

A Homebrew tap (`brew install teramind-org/tap/teramind`) is generated by CI as a fast follow.

### 7.3 First-run (`teramind init`)

1. Create `~/.local/share/teramind/{,pgdata/,raw/,logs/,inbox/,dead_letter/}` and `~/.config/teramind/`.
2. Download embedded Postgres binary (~50 MB) for current OS/arch via `postgresql_embedded`, cached for reuse.
3. Apply migrations.
4. Write default `~/.config/teramind/config.toml` (redaction rules, autorecall on, prune policy off, search blend defaults).
5. Print socket path and Postgres data dir.

### 7.4 Plugin install (`teramind claude install`)

1. Locate Claude Code config root (`$CLAUDE_HOME` or `~/.claude/`).
2. Create `~/.claude/plugins/teramind/`; copy the bundled `plugins/claude/` template.
3. Patch `plugin.json` with absolute paths to local `teramind-hook` and `teramind-mcp` binaries.
4. Run `teramind-hook --selftest` to confirm hooks are reachable.
5. Print "open Claude Code; run a session; then `teramind sessions` to confirm capture."

`teramind claude uninstall` removes `~/.claude/plugins/teramind/` and nothing else; user data untouched.

### 7.5 Daemon supervision

- **Lazy-spawn:** first `teramind-hook` invocation tries the socket with a 50 ms deadline; on failure `exec`s `teramindd --background --detached` and retries once after 250 ms. On second failure, the hook writes to `inbox/` and exits 0.
- **PID file:** `~/.local/share/teramind/teramindd.pid`. `teramind status` checks PID liveness; `teramind stop` sends `SIGTERM` and waits up to 10 s before `SIGKILL`.
- **No OS-service auto-registration in v1.** A `teramind service install` subcommand for launchd / systemd-user / Windows Service is a follow-on.
- **Logs:** `tracing-appender` with daily rotation, 14-day retention, written to `~/.local/share/teramind/logs/`.
- **Crash recovery:** the daemon writes a heartbeat file every 30 s. On startup, if the prior heartbeat is < 60 s old and the PID is dead, the daemon logs `unclean_shutdown=true` and continues. Postgres recovery is via its own WAL.

### 7.6 Update and uninstall

- `teramind self update` downloads the latest release, verifies, swaps the binary atomically (write-new + rename), and re-runs migrations. Postgres binary upgrades only happen on major version bumps with explicit `--upgrade-postgres`, to avoid silent data-dir incompatibility.
- `teramind uninstall` removes the PATH entry / symlink. `teramind uninstall --purge` also removes the data dir, config dir, and plugin dir.

### 7.7 Cross-platform notes

- Path separators and named-pipe naming are isolated in a `cfg`-gated transport adapter in `teramind-ipc`.
- macOS Gatekeeper: release binaries are notarized in CI (Apple Developer ID). Without notarization, Gatekeeper blocks on first run.
- Windows Defender / SmartScreen: v1 ships unsigned; signed binaries (Authenticode) ship in v1.1. Documented in release notes.

## 8. Observability, error handling, telemetry

The capture-side error matrix is the table in Section 5. This section consolidates what the user sees.

- **Logs:** `tracing` crate, JSON output by default, `TERAMIND_LOG=debug`-style env override. 14-day retention.
- **Metrics:** in-process counters/gauges, exposed via `teramind status --format=json`. No Prometheus endpoint in v1.
- **`teramind doctor`:** one-shot health check (daemon up, PG reachable, plugin installed, hooks executable, disk headroom, last 5 ingest events succeeded, latest storage sample). Output is pasteable for bug reports. Once the L5 baseline exists, it also displays the local corpus's nDCG@10.
- **`teramind sessions [--last N]`:** lists recent sessions with capture stats (turns captured, diffs captured, drops, gaps).
- **`teramind status`:** daemon state, PG state, queue depth, last 24h drop count, latest storage sample.
- **Audit log:** every IPC call writes a structured access entry (`audit.log`) with `{ts, caller_pid, caller_exe, method, request_id, outcome}`. Read-only queries are sampled at 1%; all writes log 100%.
- **Telemetry:** **strictly local in v1.** No outbound calls except by user-installed Claude. Update checks (`teramind self update`) are opt-in and only hit `get.teramind.dev/releases.json`. No crash phone-home. No anonymous usage stats.
- **Dead-letter directory:** `~/.local/share/teramind/dead_letter/`. A durable buffer surfaced by `teramind doctor`. Non-zero count raises a warning in `teramind status`.

## 9. Testing strategy

Five test layers; TDD throughout implementation.

```
┌─────────────────────────────────────────────────────────────────┐
│ L5: Search effectiveness benchmark   100 queries, 2k sessions    │
├─────────────────────────────────────────────────────────────────┤
│ L4: E2E (real Claude Code)           ~10 tests, nightly          │
├─────────────────────────────────────────────────────────────────┤
│ L3: Integration (daemon + real PG)   ~80 tests                   │
├─────────────────────────────────────────────────────────────────┤
│ L2: Component (per-crate, real PG)   ~150 tests                  │
├─────────────────────────────────────────────────────────────────┤
│ L1: Unit (pure logic, no I/O)        ~300 tests                  │
└─────────────────────────────────────────────────────────────────┘
```

### 9.1 L1 — Unit (pure logic, no I/O)

- `teramind-core` types: serialization round-trips, redaction regex against a corpus of secret samples, error variant constructors.
- Diff/excerpt math: hunk extraction, line-window selection, sha256 hashing, attribution decision logic.
- Ranking blend math.
- Path normalization across platforms.

### 9.2 L2 — Component (per-crate, real embedded Postgres)

- `teramind-db`: every repository method against a fresh PG instance (isolated schema per test via random schema name).
- `teramind-ipc`: roundtrip every JSON-RPC method on an in-memory transport stub.
- `teramind-mcp`: every MCP tool against a fake `IpcClient`.
- Redaction: `proptest` fuzz, asserting no rejected secret ever appears in post-redaction output.

### 9.3 L3 — Integration (full daemon, real PG, no Claude)

- **Capture E2E:** test harness simulates the full hook event stream by `exec`ing `teramind-hook` against a running `teramindd`. Assert resulting rows.
- **FS watcher attribution:** synthetic `PostToolUse` for `Edit` → file modification → `file_diffs` row with `attribution='agent'` within 1 s. Same without tool event → `attribution='human'`.
- **Search ranking:** seed DB with a fixture corpus and assert top-K matches on canonical queries (regression-pinned).
- **Grep fallback:** kill embedded PG; assert results returned with `degraded: true` and the right hits.
- **Backpressure:** 50k events at line rate; hook never blocks > 10 ms; `ingest_drops_total` accurate; JSONL complete.
- **Crash recovery:** SIGKILL mid-ingest; restart drains inbox; no double-count.
- **Migrations:** forward `v1 → v2 → ... → vN` on populated DB; lossy migrations fail loud.
- **Cross-platform IPC:** the same suite on macOS, Linux, Windows CI runners.

### 9.4 L4 — E2E with real Claude Code (nightly)

- Fresh install → `teramind claude install` → scripted Claude session using Edit/Bash → assert trace + diffs persisted.
- Auto-recall: open a session in a cwd with prior history → assert SessionStart digest contains expected snippets.
- MCP recall: scripted prompt that triggers `mcp__teramind__search` → assert daemon served it and the agent received structured results.
- Uninstall round-trip: `teramind claude uninstall` leaves nothing in `~/.claude/plugins/`.

### 9.5 L5 — Search effectiveness benchmark

Treats search quality as a measurable artifact with CI regression gates.

**Corpus location:** `benches/search-eval/`

```
benches/search-eval/
├── corpus/
│   ├── sessions.jsonl           ── 2000 synthetic sessions, hand-curated
│   ├── turns.jsonl
│   ├── tool_calls.jsonl
│   └── file_diffs.jsonl
├── qrels.toml                   ── query-relevance judgments (graded 0/1/2)
├── queries.toml                 ── 100 labelled queries
├── baseline.json                ── current main-branch metrics (committed)
└── README.md                    ── corpus authoring guide
```

**Query intent classes** (≥20 queries each):

| Class | Example | Expected hit shape |
|---|---|---|
| Natural language | "how did we fix the JWT expiry bug" | `Turn` |
| Stack trace | `thread 'main' panicked at serializer.rs:142` | `ToolCall` (Bash/test output) |
| Code snippet | `if let Some(x) = self.headers.get("Authorization")` | `FileDiff` (pre_excerpt) |
| Tool-typed | `tool:Edit path:src/parser.rs` | `FileDiff` |
| Symbolic / file path | `serialize_with_options` or `crates/teramind-core/src/redact.rs` | `Turn` or `FileDiff` |

**Metrics (per class and overall):**

- **nDCG@10** — headline metric.
- **MRR**.
- **Precision@5, Precision@10**.
- **Recall@10**.

**Computation:** the `teramind-search-eval` binary (CI-only, behind a feature flag) loads the corpus into a throwaway DB, runs every query, computes metrics, writes `eval-results.json` and a Markdown scorecard. Runtime budget: < 2 minutes.

**Regression gates** (enforced on PRs touching `crates/teramind-db/src/search/` or `crates/teramindd/src/services/search.rs`):

| Gate | Threshold |
|---|---|
| nDCG@10 (overall) | Must not drop more than 2 pp vs. `main` baseline |
| nDCG@10 (any single class) | Must not drop more than 5 pp vs. `main` baseline |
| MRR (overall) | Must not drop more than 0.03 absolute |
| Eval p95 latency | Must not exceed 3 s per query |

Baseline is the `main`-branch result, recomputed on each merge and committed to `benches/search-eval/baseline.json`. A PR may rebaseline by including an `[eval-baseline-update]` tag; reviewers can inspect the new numbers in the diff.

**Corpus growth:** v1 ships with the synthetic 2k-session / 100-query corpus. A `teramind eval contribute` CLI subcommand (post-v1) lets real users export an anonymized, redacted slice as a contribution candidate.

**`teramind doctor` integration:** once the L5 baseline exists, `doctor` reports the local-corpus nDCG@10 so users can see their own data's health relative to the benchmark.

### 9.6 Property-based and fault-injection tests

- `proptest` on: redaction (no secret survives), excerpt math (hunk windows always valid), ordinal assignment (no duplicates under concurrent fake hooks), diff parser round-trip.
- `teramind-test-harness` binary (`#[cfg(feature = "test-harness")]`, not shipped) exposes a side door for: killing PG, simulated disk-full, saturating the ingest channel, breaking inbox permissions.

### 9.7 Performance budgets (criterion benches, CI-enforced)

| Path | Budget |
|---|---|
| `teramind-hook` cold start + IPC notify + exit | p99 < 15 ms |
| Daemon ingest (event in → committed in PG) | p99 < 50 ms |
| Search blended query (10k-session fixture) | p99 < 5 s, target ≤ 800 ms |
| FS watcher: file save → `file_diffs` row written | p99 < 1 s |

### 9.8 CI matrix

`macos-arm64`, `macos-x86_64`, `ubuntu-22.04-x86_64`, `ubuntu-22.04-arm64`, `windows-2022-x86_64`.

- L1 + L2 on every PR.
- L3 on PR with `[full]` label, and on every push to `main`.
- L4 nightly and on release candidates.
- L5 on every PR touching search paths; otherwise weekly.

## 10. Open follow-on specs

These are the next planned designs that depend on Teramind Core:

| # | Spec | Depends on this spec via |
|---|---|---|
| #2 | Session summarizer (background LLM worker writing wiki pages) | `sessions`, `turns`, `tool_calls`, `file_diffs` |
| #3 | Pattern → skill codifier (mines repeated patterns into skills) | All trace tables + `skills` |
| #4 | Team sync server (auth, multi-machine replication, propagation) | The IPC contract becomes the wire shape; daemon-server symmetry |
| #5 | Codex / Cursor / Hermes / Pi connectors | The agent-agnostic schema in this spec |
| #6 | Web UI / dashboard | Read-only views over the schema |
| #7 | `pgvector` / semantic search | Adds to the search service without changing the IPC contract |

## 11. Glossary

- **Turn** — one logical exchange: a user prompt and the assistant's response (including any tool calls within it).
- **Session** — one agent invocation (a single Claude Code run, terminated by a `Stop` hook or idle timeout). Sub-agent sessions get their own row with `parent_session_id` set.
- **Capture** — the act of recording trace events into Postgres + JSONL.
- **Recall** — structured prior-context lookup, used by the model mid-session.
- **Auto-recall** — context digest injected by the `SessionStart` hook when a session opens in a cwd with prior history.
- **Attribution** — for a file diff, whether the change is `agent` (caused by a tool call) or `human` (caused by the user editing manually outside a tool call).
- **Grep fallback** — degraded search mode that runs `grep` against JSONL shadow logs when Postgres is unreachable.
- **JSONL shadow log** — the redundant, durable, append-only event log on disk that backs the grep fallback and any future re-import.
