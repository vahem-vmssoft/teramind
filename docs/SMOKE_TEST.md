# Smoke Test Report — Teramind End-to-End Shakedown

**Date:** 2026-06-27  
**Tester:** Claude Code (automated)  
**Binary versions:** Built from `main` at `8e93da1`, installed via symlinks at `~/.local/bin/`  
**Environment:** Linux, fastembed:nomic-embed-text-v1.5, ollama:qwen2.5:3b, no team mode  
**DB state:** 32 sessions · 120 turns · 2219 tool calls · 69 file_diffs · 189 embeddings · 25 wiki_pages

---

## 1. Stand It Up

### What I did

Binaries were already built (`cargo build --workspace --release`) and symlinked:

```sh
ln -sf "$PWD/target/release/teramind"     ~/.local/bin/teramind
ln -sf "$PWD/target/release/teramindd"    ~/.local/bin/teramindd
ln -sf "$PWD/target/release/teramind-hook" ~/.local/bin/teramind-hook
ln -sf "$PWD/target/release/teramind-mcp" ~/.local/bin/teramind-mcp
```

Daemon was already running (uptime 5700s). Cold start path not verified in this run.

### Config in place

| File | Present | Notes |
|---|---|---|
| `~/.config/teramind/config.toml` | ✓ | Default values |
| `~/.config/teramind/embed.toml` | ✓ | `provider = "fastembed"`, `model = "nomic-embed-text-v1.5"` |
| `~/.config/teramind/search.toml` | ✓ | `fts=0.6, semantic=0.5` |
| `~/.config/teramind/summarize.toml` | ✓ | `provider = "ollama"`, `model = "qwen2.5:3b"` |

### Startup output

```
uptime           : 5425s
pg connected     : true
ingest queue     : 0
ingest drops     : 0
```

`teramind doctor` passes all local checks. Embedded Postgres runs on `/tmp/.s.PGSQL.54817` (password: `teramind`, user: `postgres`).

### Undocumented setup steps encountered

- README says "Internet on first run" to download embedded Postgres — not documented that `~/.theseus/` is where the PG binary lands, or that the psql binary lives at `~/.theseus/postgresql/16.13.0/bin/psql`. No `psql` on PATH by default.
- `teramind status --format=json` does not include `pg_url` in its output. README tells you to use `teramind status --format=json | jq -r '.pg_url'` — that field doesn't exist.

---

## 2. Feature Inventory

| Feature | How to exercise | Expected result |
|---|---|---|
| Daemon lifecycle | `teramind start / stop / restart / status` | Process management, embedded PG |
| Health report | `teramind doctor` | Shows all subsystem health |
| Search (FTS+semantic) | `teramind search "<query>"` | Ranked hits from `traces_fts` + pgvector |
| Search (grep fallback) | `teramind search "<query>" --grep` | Live scan of raw JSONL |
| Search (JSON output) | `teramind search "<query>" --json` | Structured JSON hits |
| Session summary | `teramind sessions show [--json]` | Markdown wiki page |
| Skills list | `teramind skills list [--filter=...]` | Skill rows |
| Skill detail | `teramind skills show <name-or-id>` | Skill body |
| Codifier observations | `teramind skills observations [--kind] [--min-freq] [--status]` | Observation rows |
| Redaction preview | `teramind redact test "<text>"` | Redacted output |
| Self-update check | `teramind self-update --check-only` | Version comparison |
| Init config | `teramind init` | Writes `config.toml` skeleton |
| Reset | `teramind reset [--purge]` | Wipes local data |
| Team share toggle | `teramind team share-set --enable\|--disable` | Writes `.teramind/team-share.toml` |
| Live feed | `teramind feed [--follow]` | WebSocket stream (team mode only) |
| Hook self-test | `teramind-hook --selftest` | Roundtrip event envelope |
| Claude capture | Open Claude session, do work, exit | Events in `sessions`/`turns`/`tool_calls` |
| Auto-recall | Start new Claude session in same dir | Digest printed to Claude stdout |
| FS watcher | Agent or human edits a file | Row in `file_diffs` with attribution |
| Summarizer | Session ≥3 turns, ≥60s → ends | Non-empty `wiki_pages` row |
| Embedding pipeline | New turn ingested | Row in `embeddings` |
| MCP recall | `mcp__teramind__recall` in Claude session | Prior context injected |
| MCP search | `mcp__teramind__search` in Claude session | Hits returned |
| MCP save skill | `mcp__teramind__save_skill` | Skill row written |
| Codifier (LLM) | `codify.toml` present, observations accumulate | Candidate skills generated |
| Team: sync server | `teramind-sync-server serve` with config | HTTP(S) server up |
| Team: invite flow | Create invite → `teramind init --team` | Device registered |
| Team: dashboard | Browser at sync server URL | Admin UI |

---

## 3. Verified / Broken / Needs-Human

---

### BROKEN — fix before shipping

#### B1. `--min-freq` filter on `teramind skills observations` is silently discarded

```
teramind skills observations --min-freq=2
# → returns 50 rows all with freq=1
```

**Root cause** — `crates/teramindd/src/services/rpc_dispatch.rs:270`:

```rust
let _ = min_freq; // status takes priority; could combine
```

The parameter is bound but never passed to the query. `list_recent` receives only `kind`, `status`, and `limit`. Every query ignores the frequency filter entirely.

**Risk:** Any tooling or UI built on this flag will silently show unfiltered results.

---

#### B2. Anthropic API key pattern missing from redactor

```
teramind redact test "sk-ant-api03-test123 and sk-ant-prod-abc"
# → sk-ant-api03-test123 and sk-ant-prod-abc   (unchanged)
```

**Root cause** — `crates/teramind-core/src/redact/patterns.rs` has patterns for AWS, GitHub, Slack, JWT, PEM, `password_kv`, and `env_secret` — but **no pattern for `sk-ant-*`** (Anthropic API keys). The `env_secret` pattern catches `ANTHROPIC_API_KEY=sk-ant-...` only if the variable name appears.

```
teramind redact test "ANTHROPIC_API_KEY=sk-ant-abc123"
# → «redacted:env_secret»  ✓ (variable name form caught)

teramind redact test "sk-ant-api03-xyz123"
# → sk-ant-api03-xyz123    ✗ (bare key not caught)
```

**Risk:** Bare Anthropic keys in captured user prompts or tool outputs are stored unredacted. This is the primary secret the product's own users are most likely to type.

---

#### B3. Ollama summarizer silently writes empty wiki_pages

```sql
SELECT count(*) FILTER (WHERE length(content) > 0) AS non_empty,
       count(*) FILTER (WHERE length(content) = 0)  AS empty
FROM wiki_pages;
-- non_empty | empty
--         2 |    23
```

23 of 25 wiki_pages have `length(content) = 0`. `teramind status` reports `summary_healthy: true` and the daemon logs no error. The `sessions show` command silently outputs an empty summary.

**Root cause** — `crates/teramindd/src/services/summarizer_worker.rs:122–130`: when `result.content` is an empty string, the code still calls `repo.upsert(...)`, increments `stats.written`, and logs "summarizer wrote wiki page". There is no guard:

```rust
// MISSING:
if result.content.is_empty() {
    warn!("summarizer returned empty content; skipping upsert");
    ...
}
```

The Ollama model likely runs out of context window silently. The health check only calls `/api/version` — it does not verify that the model actually produces output.

**Risk:** Feature appears to work (`summary_healthy: true`, `sessions show` succeeds) but the summaries are empty. Users relying on session wiki pages get nothing.

---

#### B4. `teramind feed` error says "team.toml perms" when the file is absent

```
teramind feed
# Error: load /home/karen/.config/teramind/team.toml — team mode required
# Caused by:
#     0: team.toml perms
#     1: stat ... No such file or directory (os error 2)
```

The error chain labels a missing-file condition as a permissions error. "team.toml perms" implies a `chmod` fix; the real fix is `teramind init --team ...`. Misleads users trying to diagnose why feed doesn't work.

---

#### B5. 22 dead-letter events not drained on daemon restart

```
ls ~/.local/share/teramind/dead_letter/ | wc -l
# 22
```

Files date from 2026-06-22 to 2026-06-26. The README states: "files there mean the daemon was unreachable when a hook fired; they drain on the next daemon start." Daemon has been running for 5700+ seconds and the files remain.

**Risk:** Events captured during any daemon gap are permanently lost.

---

#### B6. 8/32 sessions (25%) have no `ended_at`

```sql
SELECT count(*) FROM sessions WHERE ended_at IS NULL;
-- 8
```

Sessions opened by Claude Code but never closed — the stop hook did not fire for 25% of sessions. These sessions have real turns and tool calls, but no session-end timestamp. The summarizer cannot process them (requires `ended_at`).

---

#### B7. Search response time is ~47–52 seconds for all queries (warm model)

```
time teramind search "redaction" --limit=3
# real 0m47.190s
```

This is not first-load latency — a second query on the same warm model took the same time. For a product advertising "searchable" session history, 47 seconds per query is unusable in an interactive workflow.

**Likely cause:** The fastembed model embedding is run inline on the search path. With `fts=0.6, semantic=0.5`, every query embeds the query string before the pgvector similarity search. The embedding call takes ~47s for a short query on this machine.

**Workaround:** `teramind search "..." --grep` returns in <1s (no embedding).

---

#### B8. `teramind status` shows `last_storage_pg_bytes: 0` but DB has ~144 MB

```json
"last_storage_pg_bytes": 0,
"last_storage_jsonl_bytes": 0,
```

The `storage_stats` table shows:
```
pg_bytes: 151437839   (≈144 MB)
jsonl_bytes: 715427566 (≈682 MB)
```

The status struct reports these as `0`. These fields are only updated by the storage-sampling background task. If the in-memory counters are not persisted across restarts and the next sample hasn't run yet, they show stale zeros. The behavior is confusing — a fresh daemon restart makes it look like no data has been stored.

---

### NEEDS-HUMAN — cannot be automated

#### H1. End-to-end Claude session capture

**Steps:**
1. Open a new Claude Code session in any directory.
2. Send at least one prompt and use one tool (e.g., `ls /`).
3. Exit.

**Expected:**
```sh
teramind sessions --last 5   # new row with non-zero turn count
```
**Why human?** Requires an interactive Claude Code session. Auto-spawning it programmatically doesn't fire the hooks.

---

#### H2. Auto-recall digest at session start

**Steps:**
1. After H1 completes (data captured in some `$PWD`), open a new Claude session in the same directory.
2. Look at the first lines of Claude's output.

**Expected:** A "Recent context for this project" digest is printed, listing recent turns and diffs.

**File ref:** `crates/teramind-mcp/src/` — the `recall` MCP tool fires on session start.

---

#### H3. FS watcher diff attribution

**Steps (per `docs/runbooks/fs-watcher-manual-smoke.md`):**
1. Inside a Claude Code session, ask it to edit a file.
2. Query: `SELECT rel_path, attribution FROM file_diffs ORDER BY captured_at DESC LIMIT 5;`

**Expected:** `attribution = 'agent'` for the agent edit; `attribution = 'human'` for a manual shell edit outside Claude.

---

#### H4. Summarizer produces non-empty output (verify B3)

**Steps:**
1. Fix the empty-content guard (B3 above) OR confirm Ollama is configured correctly.
2. Run a session ≥3 turns, ≥60s, then exit.
3. Wait ~30s, then: `teramind sessions show`.

**Expected:** Non-empty Markdown wiki page.

**Current state:** 23/25 summaries are empty (verified by DB query). The health check passes regardless.

---

#### H5. Team mode: sync server, invite flow, dashboard

Requires:
- A running `teramind-sync-server` with a config TOML.
- TLS cert or `--insecure-allow-http` flag.
- An admin password set via `teramind-sync-server admin-password`.
- `teramind init --team --server=<url> --invite=<code>`.

Not exercised in this run. See `docs/runbooks/sync-server-deploy.md`.

---

#### H6. MCP tools (`mcp__teramind__recall`, `mcp__teramind__search`, `mcp__teramind__save_skill`)

Requires an active Claude Code session with the plugin loaded. The `teramind-mcp` binary is built and healthy, but MCP tool invocations can only be triggered by a running Claude session.

---

#### H7. Codifier (LLM skill generation from observations)

`teramind doctor` reports `codifier: disabled (no codify.toml)`. Requires creating `~/.config/teramind/codify.toml` with a provider + model and accumulating enough repeated tool-chain observations.

---

### VERIFIED WORKING

| # | Feature | Command | Actual output |
|---|---|---|---|
| V1 | Daemon running | `teramind status` | `uptime: 5425s, pg connected: true` |
| V2 | Status JSON | `teramind status --format=json` | Correct JSON shape with all fields |
| V3 | Doctor | `teramind doctor` | All checks pass, prints subsystem table |
| V4 | Version | `teramind version` | `teramind 0.1.0` |
| V5 | Search (ranked) | `teramind search "claude code" --limit=5` | 5 hits, scores, session IDs |
| V6 | Search (JSON) | `teramind search "claude code" --json` | `{"hits":[...], "degraded":false, "took_ms":49024}` |
| V7 | Search (grep) | `teramind search "database migration" --grep` | Live JSONL scan, <1s |
| V8 | Sessions show | `teramind sessions show` | Correct Markdown for most recent session |
| V9 | Sessions show JSON | `teramind sessions show --json` | `{"content":"...","session_id":"..."}` |
| V10 | Skills list (empty) | `teramind skills list` | `(no skills)` — correct when none exist |
| V11 | Skills observations | `teramind skills observations` | 65 rows, correct columns |
| V12 | Skills obs limit | `teramind skills observations --limit=3` | Exactly 3 rows |
| V13 | Redact (env var form) | `teramind redact test "ANTHROPIC_API_KEY=sk-ant-abc"` | `«redacted:env_secret»` |
| V14 | Hook self-test | `teramind-hook --selftest` | `selftest OK`, prints event envelope |
| V15 | Team share-set | `teramind team share-set --enable` | Writes `.teramind/team-share.toml` |
| V16 | Self-update (offline) | `teramind self-update --check-only` | DNS failure (expected; no `get.teramind.dev`) |
| V17 | Embedded Postgres | DB query | 32 sessions, 120 turns, 2219 tool calls |
| V18 | Embedding pipeline | `status --format=json` | `embedding_healthy: true`, backlog=0 |
| V19 | Dead-letter report | `teramind doctor` | Shows `dead_letter: 22 files` (B5) |
| V20 | Init (safe) | Reads `config.toml` without `teramind init` | Daemon uses built-in defaults |

---

## 4. Integration Test False Confidence Audit

### Summary

The test suite has **32 integration tests** across `teramind-sync-server/tests/`, `teramind-hook/tests/`, and `teramindd/tests/`. All use **real embedded Postgres** via `teramind_db::testing::fresh_pool()` — no mocked databases. However, several tests provide false confidence by checking only status codes or row existence without validating field values.

---

### FALSE1. `--min-freq` filter handler: `let _ = min_freq` — shipped to prod untested

The code at `crates/teramindd/src/services/rpc_dispatch.rs:270` silently discards `min_freq`. There is **no test** that verifies the filter actually reduces the result set. Any test that calls `SkillsObservations { min_freq: 2 }` would pass even if all returned rows have `freq=1`.

---

### FALSE2. Skill approval only checks EXISTS, not body or source integrity

**`crates/teramind-sync-server/tests/admin_candidate_reject_then_approve.rs`**

```rust
assert_eq!(r.status(), 200);
let skill_id = body["skill_id"].as_str().expect("...");
assert!(!skill_id.is_empty());
// ← no query: SELECT body, source FROM skills WHERE id=$skill_id
// ← no check: candidate.status = 'approved', reviewed_at IS NOT NULL
// ← no check: skill.body = candidate.body
```

Approval could write a corrupted skill body or leave `candidate.status` unchanged without failing the test.

---

### FALSE3. Ingest deduplication checks response counts, not DB state

**`crates/teramind-sync-server/tests/ingest_endpoint.rs:158–174`**

```rust
assert_eq!(s["accepted"].as_i64().unwrap() + s["duplicates"].as_i64().unwrap(), 1);
// ← no query: SELECT count(*) FROM ... WHERE client_event_id = $1
```

If a bug causes two rows with the same `client_event_id`, the test passes because the *response* claims one duplicate.

---

### FALSE4. Hook test checks row count, not field values

**`crates/teramind-hook/tests/happy_path.rs:32–131`** (`hook_session_start_persists_to_postgres`)

```rust
let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id=$1")
    .fetch_one(pool.pg()).await.unwrap();
assert_eq!(count, 1);
// ← no check: session.cwd = "/work" (what was sent)
// ← no check: session.agent_kind = "claude_code"
// ← no check: session.started_at ≈ event.ts
```

The session row could have `cwd = ""` and the test would pass.

---

### FALSE5. WebSocket auth not tested for rejected connections

**`crates/teramind-sync-server/tests/admin_activity.rs`**

The happy path (valid cookie → events received) is tested. Missing:
- WebSocket without cookie → should be 401/403
- WebSocket with expired cookie → should be rejected
- WebSocket with tampered cookie → should be rejected

---

### FALSE6. Proof field mismatches not tested

**`crates/teramind-sync-server/tests/auth_middleware.rs`**

Tested: missing auth, replayed JTI. **Not tested:**
- `htm` mismatch (proof says POST, request is GET)
- `htu` mismatch (wrong URL in proof)
- `ath` mismatch (token hash doesn't match bearer)
- `bsh` mismatch (body hash doesn't match actual body)
- Timestamp in the future (`iat > now`)
- Stale timestamp (`now - iat > 60s`)

These fields are validated in `crates/teramind-sync-server/src/proof.rs:85–104` but no integration test triggers a failure path.

---

### FALSE7. Five admin endpoints have zero test coverage

| Endpoint | Handler | Test |
|---|---|---|
| `POST /admin/members/:id/revoke` | `members.rs:60` | ❌ none |
| `POST /admin/invites/:id/revoke` | `members.rs:179` | ❌ none |
| `PATCH /admin/candidates/:id` | `candidates.rs:232` | ❌ none |
| `GET /admin/observations/:id` | `observations.rs:50` | ⚠️ happy path only |
| `GET /admin/members/:id/devices` | `members.rs:74` | ⚠️ happy path only |

---

### FALSE8. Time-based test is flaky

**`crates/teramind-sync-server/tests/admin_quality_since.rs:67–149`**

Uses `tokio::time::sleep(Duration::from_millis(20))` between inserts to establish a time cutoff. Under CI load or slow clock, the 20ms gap between DB `NOW()` and `OffsetDateTime::now_utc()` may not hold, causing the "before" and "after" buckets to merge.

---

### FALSE9. Summarizer test does not check for empty content

**`crates/teramindd/tests/summarizer_ollama.rs`** and **`summarizer_mock.rs`**

No test asserts that `wiki_pages.content` is non-empty after summarization. This is why B3 shipped undetected — the mock provider can return `content: ""` and every test passes.

---

### FALSE10. `summary_written_total` counter resets to 0 on daemon restart

`teramind status --format=json` shows `"summary_written_total": 0` despite 25 wiki_pages in the DB (2 non-empty). This is an in-process counter that is not persisted. The field name implies "total since installation" but actually means "total since last daemon start." Operator monitoring based on this counter will miss historical writes.

---

## Recommendations

**Fix now:**
1. **B1** — pass `min_freq` to `list_recent` query (`rpc_dispatch.rs:270`)
2. **B2** — add `sk-ant-[A-Za-z0-9_-]{90,}` pattern to `patterns.rs`
3. **B3** — add `if result.content.is_empty() { warn!(...); continue; }` in `summarizer_worker.rs`
4. **B4** — fix error message: distinguish "missing" from "unreadable" in `team.toml` loader
5. **B5** — investigate why inbox drain doesn't process dead-letter files on restart

**Investigate before shipping:**
- B6: Why does the stop hook miss 25% of sessions?
- B7: Search latency — verify whether 47s is from embedding or pgvector; add `--no-semantic` shortcut
- B8: Clarify `last_storage_pg_bytes` semantics in status output

**Add tests for:**
- `min_freq` filter actually filters (FALSE1)
- Summarizer rejects empty content (FALSE9)
- Skill body/source integrity after approval (FALSE2)
- At least one proof field mismatch (FALSE6)
- Both revoke endpoints (FALSE7)
