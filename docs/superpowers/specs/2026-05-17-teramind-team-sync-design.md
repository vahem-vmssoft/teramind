# Teramind Team Sync — Design Spec

- **Status:** Approved (brainstorming complete; pending implementation plan)
- **Author:** Vahe Momjyan
- **Date:** 2026-05-17
- **Scope:** Spec #4 of the Teramind product roadmap. Adds an opt-in team-mode deployment that moves storage + search + summarization onto a central server while keeping the local daemon a thin capture client.

---

## 1. Background and motivation

Teramind v1.0 (Plans A–H) shipped a complete single-user, single-machine knowledge substrate: capture every Claude Code session, search prior sessions, semantically retrieve, summarize at session end, surface everything to the agent via MCP. It works exceptionally well for a solo developer. It does *not* solve the problem that motivated the project in the first place: **teams on different continents independently re-discovering the same answers to the same problems**, because each install is its own island.

The recurring scenario is concrete: a development company is porting open-source software to OpenVMS x86. Team A on one continent ports OpenSSL; three months later Team B on another continent starts on rsync. Both projects hit identical `configure.ac` autoconf failures around missing `fork()`, identical patches to `#ifdef __VMS` blocks, identical DCL command-procedure scaffolding. Team A's traces — the conversations, the diffs, the prior patches — live on a laptop in Berlin. Team B in Tokyo can't see them.

This spec adds **team mode**: an opt-in deployment shape that points local daemons at a shared central server. Captures forward to the server; search, recall, and the MCP surface all query the server. Live propagation pushes new wiki pages and ended sessions to subscribed daemons in real time. The agent itself becomes the privacy negotiator — on first use in a project, it asks the user whether to share, then persists the decision.

Local-first install remains unchanged for solo developers. Team mode is opt-in via a separate install flow.

## 2. Goals and non-goals

### 2.1 In scope (v1.0)

- A new `teramind-sync-server` binary: HTTPS API server backed by external Postgres, hosts the same `embedding_worker` / `summarizer_worker` / search code as the local daemon, exposes invite admin via CLI subcommands.
- A new `teramind init --team --server=... --invite=...` install flow that produces a thin-client local daemon (no embedded Postgres, no embedding worker, no summarizer worker — those run on the server).
- A `team_sync` service inside the local daemon that tails the JSONL shadow log and forwards captured events to the server with persisted offset.
- An MCP proxy: `teramind-mcp` keeps its stdio interface to Claude Code but forwards each tool call (`search` / `recall` / `save_skill` / `wiki`) over HTTPS to the server.
- Authentication via invite codes redeemed for long-lived device tokens, with DPoP-style request signing so a stolen bearer token alone fails at the server.
- Per-project privacy: `.teramind/team-share.toml` opt-in marker; on first session in an unset project the hook injects a notice asking the agent to prompt the user, persisted via `mcp__teramind__team_share_set`.
- Read-path fallback: server unreachable → search/recall fall back to local grep over JSONL with `degraded: true`.
- Live propagation: `GET /v1/events` WebSocket subscription delivers `SessionEnded` / `WikiPageReady` / `SkillSaved` events to subscribed daemons.
- A `teramind feed` CLI that streams the WebSocket-delivered events as a human-readable log.
- Search scope: defaults to all team data; `--mine` and `--user=<email>` filters narrow it.
- Server deployment as a single Rust binary against operator-provided Postgres, with a `docker-compose.yml` for quick starts.
- `teramind doctor` extended with team-mode health surfaces.

### 2.2 Explicit non-goals (deferred to follow-on revisions)

- OAuth / SSO / OIDC redemption (v1.1; invite codes only in v1.0).
- Hardware-backed signing keys (Secure Enclave on macOS, TPM on Linux/Windows) — v1.1.
- Server-side hard deletion endpoint (`teramind forget`) — v1.1.
- Multi-tenancy: one server hosts multiple isolated teams — v1.2. Schema is designed to be forward-compatible.
- Web admin UI — v1.1 (read-only first, then management).
- Federation across servers — v2.
- End-to-end encryption between local daemon and server — v2+ (the server can currently read the data; trust the server like you trust a self-hosted GitLab).
- Real-time co-debugging features (live cursors, shared sessions) — out of scope entirely.
- Sliding-token rotation for theft detection — DPoP is sufficient in v1.0; rotation in v1.1 if data shows it's needed.

### 2.3 Success criteria

1. A team admin runs `teramind-sync-server serve` (against a Postgres they provide), issues invites via `teramind-sync-server invite create --email alice@acme.dev`, and the developer can `teramind init --team --server=https://teramind.acme.dev --invite=TM-...` in under 60 seconds end-to-end on a fresh machine.
2. Solo developers running `teramind init` (no flags) continue to get the v1.0 local-first experience exactly as Plans A–H described. Team mode is opt-in only.
3. On a fresh project, the developer's first Claude session sees an agent-driven prompt asking whether to share with the team; their answer is persisted to `.teramind/team-share.toml` and used silently for every future session in that project.
4. Two developers on different machines, sharing a project that is marked for team sync, see each other's session summaries surface in `mcp__teramind__search` and in `SessionStart` auto-recall digests within seconds of session end.
5. A stolen `team.toml` alone (without the matching `team-key`) fails every request at the server with `403 invalid_proof`.
6. When the server is unreachable, `teramind search` falls back to grep over local JSONL and continues to return recent local results with `degraded: true`. Captures keep flowing to local JSONL and drain to the server when it recovers.

## 3. High-level architecture

```
╔════════════════════ developer machine A ═══════════════════╗
║                                                             ║
║  Claude Code <─stdio─> teramind-mcp ─────┐                  ║
║      │ hook                              │ HTTPS+JSON-RPC   ║
║      ▼                                   │                  ║
║  teramind-hook ─UDS─> teramindd ─┐       │                  ║
║                          │ JSONL │       │                  ║
║                          ▼ tail  ▼       ▼                  ║
║                       team_sync ──HTTPS─▶│                  ║
║                                          │                  ║
║                       team_events ◄─WS───┤                  ║
╚══════════════════════════════════════════│══════════════════╝
                                           │
                                           ▼
                       ╔═══════ teramind-sync-server ═══════╗
                       ║                                     ║
                       ║   axum HTTPS handler                ║
                       ║     │   ├─ POST /v1/auth/redeem     ║
                       ║     │   ├─ POST /v1/rpc             ║
                       ║     │   ├─ POST /v1/ingest          ║
                       ║     │   └─ GET  /v1/events (WS)     ║
                       ║     │                               ║
                       ║   auth middleware (bearer + DPoP)   ║
                       ║     │                               ║
                       ║     ▼                               ║
                       ║   shared services (reused libs)     ║
                       ║     ├─ IngestService                ║
                       ║     ├─ SearchService                ║
                       ║     ├─ embedding_worker             ║
                       ║     ├─ summarizer_worker            ║
                       ║     ├─ orphan_sweeper               ║
                       ║     └─ event_bus (broadcast)        ║
                       ║                                     ║
                       ║   ──────────── Postgres ──────────  ║
                       ║   (operator-provided; external)     ║
                       ║   pg_trgm + pgcrypto + pgvector     ║
                       ║   schema: A–H + team-mode tables    ║
                       ╚═════════════════════════════════════╝
                                           ▲
                                           │ same RPC
╔═══════════════════ developer machine B ══│══════════════════╗
║                                          │                  ║
║   (same shape as machine A)              │                  ║
║                                                              ║
╚══════════════════════════════════════════════════════════════╝
```

### 3.1 Layer responsibilities (delta over local-first)

| Layer | Local-first (Plans A–H) | Team mode (this spec) |
|---|---|---|
| Hook ingest | Captures → local PG via daemon | Captures → JSONL → forwarder → server |
| Embedded PG | Runs locally; canonical store | Not used. Forwarder ships to server PG |
| `embedding_worker` | Runs in local daemon | Runs on server |
| `summarizer_worker` | Runs in local daemon | Runs on server |
| `SearchRepo` | Queried locally | Proxied over HTTPS RPC |
| `traces_fts` | Local materialized view | Server-side materialized view |
| MCP server | stdio; local IPC backend | stdio; HTTPS backend |
| Auto-recall (hook SessionStart) | Local IPC `do_auto_recall` | HTTPS `do_auto_recall` against team corpus |
| `fs_watcher` | Local; emits `file_diffs` to local PG | Local; emits `file_diffs` to JSONL → forwarder → server |
| `Redactor` | Local; runs before persist | Local; runs before *ship* (same code, same rules) |

### 3.2 Two install modes

| Command | Result |
|---|---|
| `teramind init` (unchanged) | Local-first install. Embedded PG, all workers local. No network egress beyond explicit cloud providers (Plan G/H). |
| `teramind init --team --server=URL --invite=CODE` | Team-mode install. No embedded PG. `team_sync` + `team_events` services spawned. MCP transport switches to HTTPS. Captures forward (subject to per-project marker / decision cache). |

The two modes are mutually exclusive on a given machine — `team.toml` exists or it doesn't. `teramind doctor` reports which mode is active.

## 4. Components and storage

### 4.1 Workspace layout (delta)

```
crates/teramind-sync-server/        NEW bin crate
└── src/
    ├── main.rs                     clap CLI: serve, invite, member, migrate, version
    ├── server.rs                   axum app, routes, state
    ├── auth.rs                     invite redemption + token verification middleware
    ├── proof.rs                    DPoP signer/verifier
    ├── event_bus.rs                in-process broadcast::Sender<TeamEvent>
    ├── handlers/
    │   ├── auth.rs                 POST /v1/auth/redeem
    │   ├── rpc.rs                  POST /v1/rpc (dispatches to shared IPC handler)
    │   ├── ingest.rs               POST /v1/ingest (batch capture upload)
    │   ├── events.rs               GET  /v1/events (WebSocket)
    │   └── health.rs               GET  /v1/health, /v1/version
    └── config.rs                   ServerConfig

crates/teramindd/                   MODIFIED
└── src/
    └── services/
        ├── team_sync.rs            NEW: tail-JSONL forwarder
        ├── team_events.rs          NEW: WebSocket subscriber
        ├── decision_cache.rs       NEW: per-session ShareDecision state
        └── transport/              NEW: RpcTransport trait + impls
            ├── local_ipc.rs        existing UDS/named pipe transport
            └── https.rs            new HTTPS+JSON-RPC transport (used in team mode)

crates/teramind-mcp/                MODIFIED
└── src/server.rs                   uses RpcTransport trait; mode-selected at startup

crates/teramind-db/                 MODIFIED
├── migrations/
│   └── 20260517000001_team_mode.sql  NEW (server-only; not applied locally)
└── src/repos/
    ├── user.rs                     NEW: UserRepo
    ├── device.rs                   NEW: DeviceRepo (includes public_key)
    └── invite.rs                   NEW: InviteRepo

crates/teramind/                    MODIFIED
└── src/commands/
    ├── init.rs                     extended with --team / --server / --invite
    ├── team.rs                     NEW: feed, share-set, share-list
    └── doctor.rs                   extended with team-mode health surfaces
```

### 4.2 Storage: schema delta

Three new tables and additive columns on `sessions` + `skills`. All changes are forward-compatible; local-first installs never apply the team-mode migration.

```sql
-- Migration: 20260517000001_team_mode.sql  (server-only)

CREATE TABLE users (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  email        text NOT NULL UNIQUE,
  display_name text,
  created_at   timestamptz NOT NULL DEFAULT now(),
  revoked_at   timestamptz
);

CREATE TABLE devices (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         text NOT NULL,
  token_hash   bytea NOT NULL UNIQUE,        -- sha256 of the bearer token
  public_key   bytea NOT NULL,                -- Ed25519 32-byte public key
  created_at   timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz,
  revoked_at   timestamptz
);
CREATE INDEX devices_user        ON devices (user_id);
CREATE INDEX devices_last_seen   ON devices (last_seen_at DESC);

CREATE TABLE invites (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code_hash       bytea NOT NULL UNIQUE,
  invited_email   text NOT NULL,
  display_name    text,
  created_by      text,
  created_at      timestamptz NOT NULL DEFAULT now(),
  expires_at      timestamptz NOT NULL,
  redeemed_at     timestamptz,
  redeemed_device uuid REFERENCES devices(id)
);
CREATE INDEX invites_email     ON invites (invited_email);
CREATE INDEX invites_expires   ON invites (expires_at) WHERE redeemed_at IS NULL;

-- Annotate existing tables with capture-side identity.
ALTER TABLE sessions ADD COLUMN user_id    uuid REFERENCES users(id);
ALTER TABLE sessions ADD COLUMN device_id  uuid REFERENCES devices(id);
ALTER TABLE skills   ADD COLUMN user_id    uuid REFERENCES users(id);
ALTER TABLE skills   ADD COLUMN device_id  uuid REFERENCES devices(id);

CREATE INDEX sessions_user        ON sessions (user_id);
CREATE INDEX sessions_user_recent ON sessions (user_id, started_at DESC);
CREATE INDEX skills_user          ON skills (user_id);
```

**Key decisions:**

- **Identity is `(user, device)`.** Email is the carrier of user identity (set by `--email` at invite creation). Devices are per-install. Revocation works at either granularity: revoke one device (lost laptop), revoke the user (offboarding).
- **Per-row annotation only on `sessions` and `skills`.** Turns, tool_calls, file_diffs, embeddings, and wiki_pages inherit attribution via `session_id` JOIN. Avoids row-bloat on the high-volume tables.
- **Tokens stored as `sha256(token)` only.** Raw bearer tokens never persist on the server.
- **Public keys stored as raw 32-byte Ed25519 bytes.** DPoP verification uses the standard EdDSA primitives from `ring`.
- **Multi-tenant forward path.** A v1.1 migration adds `team_id` columns to `users`/`devices`/`invites` and backfills to a single implicit team. v1.0 servers behave as one team.

**Storage estimate (20 engineers, 1 year of heavy use):** ~800 MB total (raw traces + embeddings + wiki + FTS index). Comfortably runs on a 1 vCPU / 2 GB / 10 GB VM. Plenty of headroom relative to the spec's 20 GB budget.

### 4.3 Existing service reuse

The server doesn't reimplement search, embeddings, or summarization. It imports them as library code:

```rust
use teramindd::services::{
    search::{do_search, do_recall, do_auto_recall, BlendWeights},
    embedding_worker::EmbeddingWorker,
    summarizer_worker::SummarizerWorker,
    orphan_sweeper::OrphanSweeper,
    ipc_server::DaemonIpcHandler,
};
```

The `DaemonIpcHandler::handle_request` function is refactored to take an optional `AuthContext`:

```rust
pub struct AuthContext {
    pub user_id:   UserId,
    pub device_id: DeviceId,
}

impl DaemonIpcHandler {
    pub async fn handle_request(
        &self,
        req: Request,
        auth: Option<&AuthContext>,
    ) -> Response { /* ... */ }
}
```

Local IPC calls pass `None`. The server's `POST /v1/rpc` handler passes `Some(AuthContext { ... })`. The handler uses the context to annotate writes and filter reads.

## 5. Authentication

### 5.1 Invite issuance (admin)

```
$ teramind-sync-server invite create \
    --email alice@acme.dev \
    --name "Alice K." \
    --expires-in 7d \
    --created-by bob@acme.dev

invite created:
  code:  TM-3F8K-9XQ2-VLPW-7TZN
  email: alice@acme.dev
  expires: 2026-05-24T16:00:00Z
```

- 16 bytes of entropy from a CSPRNG, rendered as `TM-XXXX-XXXX-XXXX-XXXX`.
- Stored on the server as `sha256(code)` only.
- One-shot: redemption flips `redeemed_at`; further attempts fail with `409 already_redeemed`.
- Default expiry 7 days; expired invites fail `410 expired`.

Companion subcommands:

- `teramind-sync-server invite list` — outstanding (unredeemed, unexpired) invites
- `teramind-sync-server invite revoke <invite-id>` — soft-revoke before redemption
- `teramind-sync-server member list` — users with their device counts + last-seen
- `teramind-sync-server member revoke-device <device-id>` — revoke one device
- `teramind-sync-server member revoke-user <user-id>` — full offboard (cascades devices)

### 5.2 Device redemption (developer)

```
$ teramind init --team \
    --server https://teramind.acme.dev \
    --invite TM-3F8K-9XQ2-VLPW-7TZN
```

The CLI:

1. Generates an Ed25519 keypair locally via `ring`. Private key → `~/.config/teramind/team-key` (mode 0600). Public key kept in memory.
2. Detects device name from `hostname` (override via `--device-name`).
3. POSTs to the server's `/v1/auth/redeem`:

```
POST /v1/auth/redeem
Content-Type: application/json

{
  "invite_code": "TM-3F8K-9XQ2-VLPW-7TZN",
  "device_name": "alice-macbook",
  "device_public_key": "<base64 Ed25519 32-byte key>"
}
```

4. Server (atomic transaction):
   - Looks up the invite by `sha256(code)`. Rejects if missing, redeemed, or expired.
   - Upserts the user by email.
   - Inserts a device row with the new token's hash and the supplied public key.
   - Marks the invite redeemed, recording the device id.
5. Server returns the redemption result. The client writes `team.toml`:

```toml
server_url    = "https://teramind.acme.dev"
user_email    = "alice@acme.dev"
user_id       = "..."
device_id     = "..."
device_token  = "tmd_v1_..."
device_name   = "alice-macbook"
redeemed_at   = "2026-05-17T16:12:34Z"
```

`team.toml` is mode 0600. The daemon refuses to read it on subsequent runs if the mode is wider — same enforcement pattern as Plan H's secrets file.

### 5.3 Per-request authentication

Every authenticated request carries two headers:

```
Authorization: Bearer tmd_v1_<token>
X-Teramind-Proof: <compact JWS (Ed25519) over the claims below>
```

DPoP claims (per RFC 9449 with our addition):

```json
{
  "htm": "POST",                                          // HTTP method
  "htu": "https://teramind.acme.dev/v1/ingest",           // request URL
  "iat": 1747765890,                                      // Unix timestamp
  "jti": "<16 hex chars>",                                // unique nonce
  "ath": "<sha256(bearer_token)>",                        // ties proof to this token
  "bsh": "<sha256(request_body)>"                         // ties proof to this payload
}
```

Signed with the device's Ed25519 private key. Encoded as JWS in compact form: `base64url(header).base64url(claims).base64url(signature)`.

**Server-side verification (middleware) for every authenticated route:**

1. Parse `Authorization` → `sha256(token)` → SELECT device. 401 if missing.
2. Check `device.revoked_at IS NULL` and `users.revoked_at IS NULL`. 403 otherwise.
3. Parse `X-Teramind-Proof`. Verify the Ed25519 signature against `device.public_key`. 403 on mismatch.
4. Check claims:
   - `htm` matches the request method; `htu` matches the request URL. 403 otherwise (cross-route replay).
   - `ath` matches `sha256(token)`. 403 otherwise (cross-token replay).
   - `bsh` matches `sha256(request_body)`. 403 otherwise (payload tampering).
   - `|now - iat| < 60s`. 403 otherwise (stale proof).
   - `jti` not in the per-device LRU replay cache. 403 otherwise. Cache size 10k entries / device; entries expire at 60s.
5. Async-update `devices.last_seen_at = now()` (non-blocking; fire-and-forget).
6. Attach `AuthContext { user_id, device_id }` to the request and continue.

### 5.4 Defense properties

An attacker who exfiltrates `team.toml` alone — by reading the file off disk or stealing a backup — cannot make any authenticated request. The bearer token alone fails the DPoP signature check because the matching private key is in `team-key`, a separate 0600 file. An attacker who steals both files together can impersonate the device — but that requires filesystem-level access to two files, not a single credential leak.

v1.1 raises the bar further by moving `team-key` into the OS keychain (macOS Keychain, Linux Secret Service, Windows Credential Manager) and, where Secure Enclave / TPM are available, generating the private key inside the enclave so it can't be exfiltrated even with root.

### 5.5 `teramind doctor` surfaces

```
team mode:    enabled (https://teramind.acme.dev)
user/device:  alice@acme.dev / alice-macbook  (last-seen 12s ago)
auth proof:   ed25519 (key at ~/.config/teramind/team-key, mode 0600 ✓)
```

When permissions slip:

```
auth proof:   ✗ team-key has insecure perms (0644); chmod 0600 to fix
```

## 6. Capture forwarding

The `team_sync` service is a new daemon service that runs only in team mode. It tails the JSONL shadow log (Plan A) and ships events to the server in batches with a persisted offset.

### 6.1 The tail-forwarder loop

```
file:    ~/.local/share/teramind/raw/YYYY-MM-DD.jsonl
offset:  ~/.local/share/teramind/raw/.sync-offset.json
batch:   32 events
flush:   every 1s or when batch is full
backoff: exponential 1s → 60s on server errors

loop {
    select! {
        new_line = jsonl_tail.next_line() => {
            buffer.push(new_line)
            if buffer.len() >= batch_size { flush() }
        }
        _ = ticker.tick(every 1s) => {
            if !buffer.is_empty() { flush() }
        }
    }
}

fn flush() {
    let (shippable, locally_kept) = partition(buffer, |e| {
        decision_cache.is_shareable(&e.session_id)
    });
    match POST("/v1/ingest", &IngestBatch { events: shippable, ... }) {
        Ok(_) => advance_offset(buffer.last().offset),
        Err(transient) => { backoff.next(); /* don't advance; retry next tick */ },
        Err(permanent) => { mark_degraded(); /* admin must look */ },
    }
}
```

**Properties:**

- **Durability.** Events are written to JSONL *before* the forwarder reads them. Crash mid-batch → restart resumes from the persisted offset. Zero event loss across daemon restarts.
- **Backpressure-free.** Capture writes to JSONL through Plan A's existing pipeline. The forwarder reads from disk. They're decoupled. Slow server doesn't slow ingest.
- **Server-offline resilience.** Server unreachable for hours → forwarder pauses with backoff. Daemon keeps capturing. When server recovers, queued events drain in batches.
- **Idempotency.** Every event already carries `client_event_id` (Plan A's deterministic UUID). The server's `/v1/ingest` dedupes by `client_event_id`. Replaying the same batch twice is safe.

### 6.2 Per-session decision cache

A `HashMap<SessionId, ShareDecision>` in memory:

```rust
enum ShareDecision {
    Pending,            // marker absent + agent hasn't answered yet → hold events
    Allowed,            // marker says share=true OR agent answered yes → ship
    DeniedKeepLocal,    // marker says share=false OR agent answered no → drop from ship queue
}
```

Population:

1. On `IngestEvent::SessionStart`, the forwarder walks up from the session's cwd toward `$HOME` looking for `.teramind/team-share.toml`.
   - Found `share=true` → `(session_id, Allowed)`.
   - Found `share=false` → `(session_id, DeniedKeepLocal)`.
   - Absent → `(session_id, Pending)` AND the `SessionStart` hook also prints the share-prompt notice into Claude's context (see §6.3 below).

2. The MCP tool `mcp__teramind__team_share_set(scope, share)` writes `.teramind/team-share.toml` and emits an `IngestEvent::TeamShareDecided { session_id, share }` over the local IPC. The forwarder catches that event, updates the cache (`Pending → Allowed | DeniedKeepLocal`). If `Allowed`, it triggers backfill (§6.4).

3. Cache evicts on `SessionEnd` or after 12 h of inactivity (LRU).

### 6.3 Agent-driven privacy prompt

When the `SessionStart` hook fires for a project that has no `.teramind/team-share.toml` (and no ancestor file either), the hook injects a notice into Claude's context (via the existing stdout-to-context path):

> *⚠️ This project at `/Users/alice/proj/foo-vms` has no Teramind team-sharing preference set. Please ask the user once: "Share captures from this project with the team?" Then call `mcp__teramind__team_share_set(scope: 'project', share: true | false)` to record their answer. Until then, captures stay local-only.*

Claude asks the user in chat (a natural conversational moment, not a modal popup). When the user answers, Claude calls the MCP tool, which writes:

```toml
share = true                  # or false
set_by = "alice@acme.dev"
set_at = "2026-05-17T16:12:00Z"
```

Future sessions in this project (any descendant directory of the marker file) skip the prompt because the marker now exists. The marker is committable to the project's repo — teams can declare "this project shares to the team" via version control.

CLI escape hatch: `teramind team share-set --enable | --disable` flips the marker without going through the agent.

### 6.4 Backfill on consent

Sessions that started with `Pending` had their events buffered locally without shipping (they wrote to JSONL but stayed below the forwarder's offset for that session). When the user answers `share=true`:

```
fn on_share_decided(session_id, decision) {
    match (cache[session_id], decision) {
        (Pending, Allowed) => {
            ship_session_backfill(session_id)
            cache[session_id] = Allowed
        }
        (Pending, DeniedKeepLocal) => {
            cache[session_id] = DeniedKeepLocal
            // events stay in JSONL but never ship
        }
        _ => /* no-op */
    }
}
```

Backfill: read the JSONL backward to the `SessionStart` line for `session_id`, then forward from there, ship every event whose session_id matches. The offset file advances to the highest shipped offset.

### 6.5 Server-side ingest endpoint

```
POST /v1/ingest
Authorization: Bearer ...
X-Teramind-Proof: ...
Content-Type: application/json

{
  "device_id": "...",
  "user_id":   "...",
  "events": [
    { "client_event_id": "...", "ts": "...", "event": { ... } },
    ...
  ]
}

→ 200 OK
{
  "accepted":  32,
  "duplicates": 0,
  "rejected":  []
}
→ 400 invalid_event_shape
→ 401 invalid_or_expired_token
→ 403 invalid_proof | device_revoked | user_revoked
→ 429 rate_limit
→ 503 server_busy
```

Server-side processing (transactional per batch):

1. Auth middleware verifies bearer + proof.
2. For each event: stamp `user_id` + `device_id` from the auth context onto the row.
3. Route to the existing `IngestService::route()` — exactly the same dispatch path Plan A uses locally. No new event types, no new handlers. Downstream handlers (e.g., `SessionEnd → wiki backlog enqueue → summarizer_worker`) publish their own `TeamEvent`s on the bus (§8.1) at their natural completion points.
4. Return summary.

### 6.6 `teramind doctor` surfaces

```
team mode: enabled (https://teramind.acme.dev)
team sync: ↑ 1,247 events / 39 batches  (last batch 4s ago)
team sync: ▢   12 events held (pending agent decision)
team sync: ✗    0 events dropped (user opted out)
```

If the server is unhealthy:

```
team sync: UNHEALTHY since 2026-05-17T16:08:14Z (events queueing in JSONL)
team sync: ↑ 1,247 events shipped / 47 events backlogged
```

## 7. MCP proxy

Claude Code's MCP integration is stdio-only. In team mode, `teramind-mcp` keeps its stdio interface to the agent but forwards each tool call over HTTPS to the server.

### 7.1 The `RpcTransport` trait

```rust
#[async_trait]
trait RpcTransport: Send + Sync {
    async fn request(&self, req: teramind_ipc::proto::Request)
        -> anyhow::Result<teramind_ipc::proto::Response>;
}
```

Two implementations:

- **`LocalIpcTransport`** (existing) — opens UDS / named pipe, sends `Request` JSON, reads `Response`. Used in local-first mode.
- **`HttpsTransport`** (new) — `POST {server_url}/v1/rpc` with bearer + DPoP proof, body is the `Request` enum as JSON, response deserializes to `Response`. Used in team mode.

`teramind-mcp` selects at startup based on the presence of `~/.config/teramind/team.toml`.

### 7.2 `/v1/rpc` endpoint

```
POST /v1/rpc
Authorization: Bearer ...
X-Teramind-Proof: ...
Content-Type: application/json

{ "method": "search", "params": { "query": "fork autoconf", "limit": 10 } }
```

The server's handler does:

```rust
async fn handle_rpc(
    Extension(auth): Extension<AuthContext>,
    Json(payload): Json<RpcEnvelope>,
) -> Response<Json<RpcResponse>> {
    let request = Request::from_envelope(payload)?;
    // Same handler the local IPC server uses.
    let response = ipc_handler.handle_request(request, Some(&auth)).await;
    Json(RpcResponse::from(response))
}
```

The handler uses `auth` to annotate writes (e.g., `save_skill` records `created_by_user_id`) and filter reads (e.g., `--mine` adds `WHERE user_id = $auth.user_id`).

### 7.3 The four MCP tools

| Tool | Request variant | Server behavior |
|---|---|---|
| `mcp__teramind__search` | `Request::Search { query, limit, json, grep }` | `do_search` against team-wide corpus; `--mine`/`--user=` filter via auth context |
| `mcp__teramind__recall` | `Request::Recall { cwd, file_paths, symbols, stack_traces, limit }` | `do_recall` against team-wide corpus |
| `mcp__teramind__save_skill` | `Request::SaveSkill { name, description, body, source_session_ids }` | Insert with `created_by_user_id = auth.user_id` |
| `mcp__teramind__wiki` | `Request::WikiLookup { session_id, cwd }` | Plan H wiki lookup against team-wide corpus |
| `mcp__teramind__team_share_set` (NEW) | `Request::TeamShareSet { scope, share }` | Local-only — writes `.teramind/team-share.toml` and emits `IngestEvent::TeamShareDecided` for the forwarder |

### 7.4 Read-path fallback

The HTTPS transport wraps RPC calls with a try-with-fallback over Plan A's `grep_fallback`:

```rust
async fn search_or_grep(&self, query: &str, limit: u32) -> SearchOutcome {
    match self.transport.request(Request::Search { ... }).await {
        Ok(Response::SearchResults(r)) => r,
        Err(e) if e.is_connection() => {
            let hits = grep_fallback::run(&self.jsonl_dir, query, limit).await
                .unwrap_or_default();
            SearchOutcome { hits, degraded: true, took_ms: 0 }
        }
        Err(_) => SearchOutcome { hits: vec![], degraded: true, took_ms: 0 },
    }
}
```

Applies to `search`, `recall`, and `wiki`. Does NOT apply to `save_skill` — writes must be durable; failure surfaces loudly with "team server unreachable; skill not saved."

### 7.5 SessionStart auto-recall in team mode

The `teramind-hook` binary already calls `Request::AutoRecall` on `SessionStart` to inject a digest into Claude's context. In team mode, the hook's `RpcTransport` is the `HttpsTransport`, so the auto-recall query runs against the team's full corpus. The hook prints the digest to stdout — same behavior, server-sourced data.

Auto-recall in team mode surfaces *the team's* recent work in this cwd's project — not just the calling developer's. That's the OpenVMS-porting scenario delivered.

A project that has not opted into team-share can still *read* team auto-recall — reading is universal; writing is opt-in.

### 7.6 Latency

| Path | Local-first | Team mode (warm conn) | Team mode (cold conn) |
|---|---|---|---|
| `mcp__teramind__search` | p95 ~50 ms | p95 ~150 ms | p95 ~400 ms |
| `mcp__teramind__wiki` (cache hit) | p95 ~20 ms | p95 ~80 ms | p95 ~350 ms |
| `mcp__teramind__save_skill` | p95 ~30 ms | p95 ~120 ms | p95 ~380 ms |

`HttpsTransport` maintains a persistent HTTP/2 connection (HTTP/3 in v1.1 if it pays off) so steady-state is "warm conn." The 50→150 ms delta adds well under 5% to agent tool-call round trips, which the model side dominates.

## 8. Live propagation

A `tokio::sync::broadcast::Sender<TeamEvent>` lives in the server's app state. Workers publish; the WebSocket handler subscribes per connection. Local daemons subscribe via WebSocket and re-publish on a local in-process bus.

### 8.1 Event types

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TeamEvent {
    SessionEnded   { session_id, user_id, cwd, ts },
    WikiPageReady  { page_id, session_id, user_id, cwd, title, ts },
    SkillSaved     { skill_id, user_id, name, ts },
}
```

Publishers:

- `IngestService::route()` on `IngestEvent::SessionEnd` → emits `SessionEnded`.
- `summarizer_worker` after `WikiRepo::upsert` → emits `WikiPageReady`.
- `Request::SaveSkill` handler after upsert → emits `SkillSaved`.

Bus capacity: 256. Lagging subscribers receive `RecvError::Lagged`, get force-disconnected, and reconnect.

### 8.2 `GET /v1/events` WebSocket endpoint

```
GET /v1/events
Sec-WebSocket-Protocol: teramind-v1
Authorization: Bearer ...
X-Teramind-Proof: ... (signs the URL; binds the connection to the device)
```

The proof is checked at upgrade time. After upgrade, no per-message auth — the connection is the session.

Server handler:

```rust
async fn ws_events(
    ws: WebSocketUpgrade,
    Extension(auth): Extension<AuthContext>,
    State(bus): State<Sender<TeamEvent>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, auth, bus.subscribe()))
}
```

Connection lifecycle:

1. On upgrade, send a `hello` frame with `server_version` and `since_ts` (so the client can decide whether to refetch missed state via polling RPC).
2. Forward every received `TeamEvent` to the socket as a JSON text frame.
3. Respond to client `Ping` with `Pong`.
4. Tear down on `Close` or any send/receive error.

### 8.3 Local-side subscription

A `team_events` service inside the local daemon spawned in team mode:

```
Connect:    wss://<server>/v1/events with proof
Reconnect:  exponential 1s → 60s with jitter
Heartbeat:  Ping every 30s; tear down on missed Pong
Republish:  received TeamEvent → local broadcast::Sender<TeamEvent>
```

### 8.4 Consumers in v1.0

Two consumers; both modest.

**1. `teramind feed` CLI:**

```
$ teramind feed
2026-05-17 16:32:14  alice@acme  SessionEnded   /proj/openvms-rsync
2026-05-17 16:31:02  bob@acme    WikiPageReady  /proj/openvms-llvm  "ASan port: vfork constraints"
2026-05-17 15:58:41  alice@acme  SkillSaved     "vms-autoconf-fork-probe"
```

Reads the local broadcast subscriber. `--follow` streams; default prints the last 50 from an in-memory ring buffer (bounded; restart loses the buffer).

**2. Auto-recall freshness signal:**

When `WikiPageReady { cwd, ... }` arrives AND a Claude session is currently active in that cwd, the daemon writes a one-line marker file. On the next `mcp__teramind__recall` or `mcp__teramind__search` call from that session, the response metadata includes `note: team produced new context for this cwd 14s ago`. Claude sees it and may re-query for the new wiki. Optional; non-load-bearing.

### 8.5 Not in v1.0

- Pushing notifications back into Claude Code mid-session — no agent-side push channel exists.
- Server-driven UI updates — no UI in v1.0.
- Custom event subscriptions / filtering per consumer.
- Replay-on-reconnect (events that fired during a connection gap are not re-sent; polling RPC still sees them via DB reads).

### 8.6 `teramind doctor` surfaces

```
team events: ws connected for 1h 14m  (received 47 events)
```

Or, when reconnecting:

```
team events: reconnecting  (last error: connection refused, retrying in 4s)
```

## 9. Configuration

### 9.1 Client: `~/.config/teramind/team.toml`

Created by `teramind init --team`. Mode 0600. Holds the bearer token and device identity (§5.2 shape).

### 9.2 Client: `~/.config/teramind/team-key`

Created by `teramind init --team`. Mode 0600. Holds the Ed25519 private key.

### 9.3 Client: per-project `.teramind/team-share.toml`

Created by `mcp__teramind__team_share_set` or by hand. Marks a project as share-eligible. Lives in the project repo and is committable.

```toml
share   = true
set_by  = "alice@acme.dev"
set_at  = "2026-05-17T16:12:00Z"
```

### 9.4 Server: `~/.config/teramind-sync-server/config.toml`

```toml
listen_addr   = "0.0.0.0:443"
database_url  = "postgres://teramind:secret@db/teramind"

[tls]
cert_file = "/etc/teramind/cert.pem"
key_file  = "/etc/teramind/key.pem"

[auth]
invite_default_expires_days = 7
proof_replay_window_secs    = 60
proof_replay_cache_size     = 10000

[ingest]
max_batch_size            = 32
max_request_body_bytes    = 10485760  # 10 MB

[embedding]
provider = "ollama"
model    = "nomic-embed-text-v2-moe"
# (same shape as Plan G's embed.toml)

[summarize]
provider = "ollama"
model    = "qwen3.6:latest"
# (same shape as Plan H's summarize.toml)
```

Server refuses to start if `[tls]` is unset unless the operator passes `--insecure-allow-http` (loud flag for dev environments only).

## 10. Testing strategy

Six layers (the existing five plus one new for multi-process team tests).

### 10.1 L1 — Unit (pure logic, no I/O)

- DPoP proof construction: `Signer::sign(claims) -> ProofHeader` deterministic for fixed key + claims; round-trips through encode/decode.
- DPoP verification: rejection axes — bad signature, stale `iat`, replayed `jti`, mismatched `htm`/`htu`/`ath`/`bsh`. Proptest: random valid claims always pass; one flipped byte in any field always fails.
- Invite code format: `generate_code()` → `TM-XXXX-XXXX-XXXX-XXXX`, 16-byte entropy; `parse_code()` round-trips; one bad char rejects.
- Decision-cache state machine: `ShareDecision` transitions; idempotent re-set; backfill triggered only on `Pending → Allowed`.
- JSONL tail offset math: rotate-at-midnight handling; correct file selection.
- Marker-file lookup walks cwd → `$HOME` with stops at `.git/` and `$HOME` boundary.

### 10.2 L2 — Component (per-crate, real Postgres)

- Team-mode migration applies; new tables exist; columns added to `sessions` and `skills`; indexes present.
- `UserRepo`, `DeviceRepo`, `InviteRepo` methods in isolation.
- Auth middleware as a tower service: missing token → 401; revoked device → 403; valid token + bad proof → 403; valid token + valid proof → 200.
- Server-side `IngestService` annotation: events arriving with `(user_id, device_id)` from auth context land correctly in `sessions.user_id` / `sessions.device_id`.
- JSONL tail-forwarder slice: file rotation across day boundary; resume from offset.

### 10.3 L3 — Integration (server subprocess + local daemons)

A new test harness `tests/team_harness.rs`:

1. Spins up an embedded Postgres in a tempdir.
2. Runs the team-mode migrations.
3. Launches `teramind-sync-server` as a subprocess on a free port.
4. Generates an invite via `invite create`.
5. Configures one or two local daemons with synthetic `team.toml` + `team-key`.
6. Drives `IngestEvent`s through the local daemons' IPC.
7. Asserts server-side rows appear.
8. Shuts everything down cleanly.

Tests at this layer:

- **End-to-end capture forwarding** (single daemon → server PG with correct `user_id`/`device_id`).
- **Decision-cache backfill** (`Pending → Allowed` ships buffered events).
- **Decision-cache deny** (`Pending → DeniedKeepLocal` keeps events local forever).
- **Per-project marker found** (no agent prompt needed; events flow immediately).
- **Server unreachable** mid-burst: forwarder buffers; server returns; forwarder drains in order.
- **DPoP proof tampering**: bad `htm` / bad `ath` / future `iat` → 403.
- **Token theft simulation**: copy `team.toml` to a second tempdir *without* `team-key`; second client gets 403 invalid_proof.
- **WebSocket subscription**: one daemon subscribes; another daemon's `SessionEnd` triggers a `SessionEnded` event delivered within 1s.
- **WebSocket reconnect**: server restart; client reconnects within 60s with jitter.
- **Two-daemon search**: daemon A captures a session; daemon B (different user) `teramind search` finds it; `--mine` from daemon B returns empty.
- **Read-path fallback**: stop the server; MCP `search` falls back to grep with `degraded: true`.

### 10.4 L4 — E2E with real Claude Code (nightly, containerized lab)

- **Live OpenVMS-style scenario**: two simulated developers, one server. Dev A ports OpenSSL (10 turns, real Claude session). Dev B starts an rsync port. Assert: B's SessionStart digest includes A's wiki; B's `mcp__teramind__search` returns hits from A's session.
- **Privacy gate**: B starts in a project with no marker. Hook injects share-prompt; Claude (scripted "yes") calls `mcp__teramind__team_share_set`. Marker written; subsequent captures flow.

### 10.5 L5 — Search effectiveness benchmark (server mode)

Plan F's L5 corpus runs against `teramind-sync-server`. Two new baselines:

- `baseline-team-mode.json` — lexical, executed via HTTPS.
- `baseline-team-mode-semantic.json` — same with `semantic_weight = 0.4`.

Gates identical to Plan F. Catches HTTPS round-trip regressions and server-side schema regressions that would silently degrade ranking.

### 10.6 Property-based and fault-injection

- Proptest: any valid `(method, url, body)` + key produces a proof that verifies; any single-byte flip in header/claims/signature fails verification.
- Replay cache LRU bound: proptest never exceeds the configured capacity.
- Network partition mid-burst (via in-process mock): drop packets for N seconds; assert no event loss; offset advances when partition heals.
- Concurrent redemption race: two simultaneous POSTs for the same invite code → exactly one succeeds, the other 409.
- Clock skew: client clock 90s ahead → server rejects with stale-iat; `teramind doctor` shows the skew.

### 10.7 Performance budgets

| Path | Budget |
|---|---|
| `team_sync` forwarder steady-state (1k events/s burst) | p99 < 50 ms per batch; no loss |
| `POST /v1/ingest` server-side per-batch (32 events) | p99 < 100 ms |
| `POST /v1/rpc` for `search` on a 10k-session corpus | p95 < 400 ms |
| WebSocket end-to-end (emit → subscriber receive) | p99 < 200 ms intra-region; 500 ms global |
| DPoP proof verification | p99 < 1 ms |
| Server cold-start to first ready response (PG already up) | < 5 s |

## 11. Rollout, dependencies, risks

### 11.1 Dependencies

- Builds on all of Plans A–H. Especially Plan A's JSONL writer (forwarder source), Plan A's IPC envelope (wire reuse), Plan G's `EmbeddingProvider` factory (server-side now), Plan H's `SummaryProvider` factory (same).
- No new follow-on specs are blocked by this. This *is* the spec that unblocks multi-developer use cases.

### 11.2 Rollout phases

1. **v1.0 (this spec)** — `teramind-sync-server` binary, invite-code auth, DPoP signing, capture forwarding, MCP proxy, WebSocket live events, `teramind feed`, per-project marker + agent-prompt privacy flow, team-mode L3 + L5 baselines.
2. **v1.0.1** — Docker image + Compose file; deployment runbook; OAuth (GitHub) as a second redemption path alongside invite codes.
3. **v1.1** — Server-side hard deletion (`teramind forget` from client; `member delete-data` from admin). Hardware-backed signing keys (macOS Keychain, Linux Secret Service, Windows Credential Manager, Secure Enclave / TPM where available). Web admin UI (read-only first).
4. **v1.2** — Multi-tenancy (one server hosts many teams). SSO / OIDC. Per-team configuration overrides for embeddings and summarization.
5. **v2** — Federation across servers; end-to-end encryption.

### 11.3 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Bearer-token-alone theft | High in a naive design → Low here | DPoP signing makes a stolen `team.toml` useless. v1.1 hardware keys raise the bar further. |
| Server outage halts the entire team's search/recall | Medium | Read-path falls back to grep over local JSONL. Capture stays local + queues in JSONL during outages, drains on recovery. |
| HTTPS round-trip latency degrades agent UX | Low | Persistent HTTP/2 keeps warm; observed delta is <100 ms; agent tool-call cost dwarfs it. |
| Privacy leak from misconfigured marker | Medium | Default-deny via agent prompt. Captures hold local until consent. Marker is checked into the project repo so PR review surfaces "this project now shares to the team." Redaction (Plan A) still runs on every event before ship. |
| WebSocket lifecycle bugs (lag drops, partial messages) | Medium | Backoff reconnect + bounded-broadcast is well-trodden. v1.0 doesn't rely on WS for *correctness* — polling RPC works when WS is down. |
| Schema drift between local-first and server modes | Low | Same `teramind-db` migrations for Plans A–H run on both sides. Team-mode adds an additive migration with no destructive changes. CI runs L2 against both schema sets. |
| Token rotation across daemon restarts | Low | Tokens are long-lived in v1.0; no rotation. Lost tokens require admin revocation. |
| Operator runs server with weak TLS or HTTP | Medium | Server refuses to start without TLS unless `--insecure-allow-http` is passed (loud flag). Operator runbook documents reverse-proxy patterns. |
| Server PG fills up | Low | Storage estimate ~800 MB / 20 engineers / year leaves enormous headroom. `teramind-sync-server storage stats` surfaces growth trend; v1.1 adds retention policy. |
| WebSocket auth replay across reconnects | Low | Each handshake mints a fresh proof bound to that URL + nonce; old proofs fail. |
| Per-session decision-cache races with rapid SessionStart | Low | Cache is keyed by SessionId, populated on the SessionStart event itself, which is always the first event for that session. Race window is bounded by JSONL ordering. |

### 11.4 Out of scope (deferred)

- Multi-tenancy / multi-team-per-server (v1.2).
- Federation across servers (v2).
- Hosted SaaS offering — v1.0 ships self-hosted; v1.2+ may add hosted.
- Web UI (v1.1+ for admin, v1.2+ for end-users).
- End-to-end encryption (v2+).
- Hard deletion / GDPR `teramind forget` (v1.1).
- Sliding-token rotation (v1.1 if data justifies).
- Real-time co-debugging features.

## 12. Glossary

- **Local-first mode** — the v1.0 install. Embedded PG, all workers local, no network egress beyond cloud providers the user explicitly opts into. Plans A–H deliver this.
- **Team mode** — opt-in install via `teramind init --team --server=... --invite=...`. Local does capture only; the server hosts storage and workers.
- **Sync server** — the central `teramind-sync-server` binary. Single Rust binary against operator-provided Postgres.
- **Invite code** — one-shot, time-limited code issued by an admin to onboard a new device. Hashed at rest.
- **Device token** — long-lived bearer token issued during invite redemption. Identifies a specific install. Hashed at rest. Revocable.
- **DPoP** — Demonstrating Proof-of-Possession (RFC 9449). Per-request Ed25519 signature binding a bearer token to a private key the device holds.
- **`team-key`** — the local Ed25519 private key file (mode 0600) used to sign DPoP proofs. v1.0 is on-disk; v1.1 moves it into the OS keychain / hardware enclave.
- **Marker file** — `.teramind/team-share.toml`. Per-project opt-in to team sharing. Commitable to the project repo.
- **Decision cache** — in-memory `SessionId → ShareDecision` map inside the local daemon. Determines whether each session's captures forward to the server.
- **Backfill** — when `Pending → Allowed` flips for a session, the forwarder ships the events that buffered locally during the pending window.
- **Read-path fallback** — when the server is unreachable, MCP read tools (search/recall/wiki) fall back to grep over local JSONL with `degraded: true`. Writes (save_skill) fail loudly.
- **Live propagation** — WebSocket-delivered `TeamEvent`s pushed from the server to subscribed daemons.
- **Identity model** — `(user_id, device_id)`. Email identifies users; each install on each device has its own device row.
