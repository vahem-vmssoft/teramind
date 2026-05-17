# Teramind Web Dashboard — Design Spec

- **Status:** Approved (brainstorming complete; pending implementation plan)
- **Author:** Vahe Momjyan
- **Date:** 2026-05-17
- **Scope:** First customer-facing UI in the post-v1.0 roadmap. An admin-only browser dashboard over the team-mode sync server. Read-only inspection of activity, the skill catalog, members + devices, and search-quality trends — plus the candidate review UX that Plan M deferred.

---

## 1. Background and motivation

Teramind v1.0 (Plans A–L) ships a complete CLI + agent surface. The MCP integration, the daemon's status reports, the team-mode forwarder, the live-event WebSocket — all of it works through stdio, sockets, and terminal commands. Operators self-host the sync server. They have no browser view over what the team is doing with it.

For solo developers the CLI is enough. For team admins it isn't. The operator who issued invites wants to see: *who connected*, *what they captured*, *which skills the codifier surfaced for review*, *whether the search-quality benchmark is still where it was a month ago*. Today that's a series of SQL queries against the sync server's Postgres. The dashboard makes those views first-class.

This spec also resolves a known v1.1 deferral from Plan M: the **skill candidate review UX**. Plan M ships with SQL-based approval (`UPDATE skill_candidates SET status='approved' WHERE id=…`). The dashboard's `/dashboard/skills` page exposes the same operation as a button click, with the candidate's body editable before approval — the v1.1 review workflow lands here in v1 of the dashboard.

The dashboard is **admin-only** in v1. Every team member doesn't get a personal view of the team's combined corpus. The audience is the operator who self-hosts the server. (A read-only viewer role for non-admin team members is a v2+ item.)

---

## 2. Goals and non-goals

### 2.1 In scope

- A React SPA served from the existing `teramind-sync-server` binary at `/dashboard/*`.
- An admin-only `/admin/*` HTTP API in the same binary, cookie-authenticated.
- Password-based admin auth — operator sets `admin_password_hash` (argon2id) in the server config; the login flow mints a signed HttpOnly session cookie.
- Four top-level views:
  - **Activity** — paginated event history + live WebSocket subscription.
  - **Skills** — catalog of authored + codified skills, plus a pending-candidate review surface with edit/approve/reject actions.
  - **Members & devices** — roster of redeemed users + their devices, with revoke buttons, plus the open-invite list with a "+ Issue invite" flow.
  - **Quality** — search-quality trends from periodic eval runs.
- A `team_event_log` table that persists every TeamEvent (so the activity view can show history beyond what the WS delivers post-connect).
- A `quality_runs` table + optional scheduled eval runner (cron-style).
- The SPA is **embedded** as static assets in the server binary via `include_dir`. A single binary ships everything; an operator updates the dashboard by upgrading the server.
- The dashboard is **disabled by default** — when `[admin]` is absent from the server config, `/admin/*` and `/dashboard/*` return 404.

### 2.2 Explicit non-goals (deferred)

- SSO / OAuth login — v1.1. Password is the only auth path in v1.
- Two-factor for the password login — v1.1.
- Read-only viewer role for non-admin team members — v2+.
- Multi-tenancy (dashboard scoping to multiple teams hosted by one server) — v1.2.
- Audit log of who approved which candidate — v1.2. (The `reviewer` column captures it; a dedicated audit view is a follow-on.)
- Editing a skill's body after promotion. Skills are append-only; to revise, codify a superseding skill.
- Multi-language UI; English only in v1.
- A mobile-native client.
- Embeddable widgets (an iframe of "latest codified skills" for the team wiki) — v2.
- A web-based "send a chat to the agent" surface. The dashboard is observational.
- An ingest-rate / error-budget / SLO dashboard. v1.2+.

### 2.3 Success criteria

1. An operator on a running sync server can: set the admin password (with the new helper subcommand) → restart the server → open `https://<host>/dashboard/` → log in → see today's activity, all skills, all members, all invites, and (if `[quality]` is enabled) yesterday's eval result. Total time from "I want a dashboard" to "I'm looking at it": under five minutes.
2. A pending skill candidate that previously required SQL approval can be approved via the dashboard with a single button click; the promotion completes before the admin's UI shows the next view.
3. The activity feed updates within 1 second of a new event firing server-side.
4. The server's release binary ships the dashboard already bundled — no extra installation step for the operator.

---

## 3. Architecture overview

The dashboard is **admin-only** and lives entirely inside the existing `teramind-sync-server` binary. Two parallel HTTP surfaces:

- The existing `/v1/*` API (bearer + DPoP signing) — unchanged. Used by CLI, daemon, MCP.
- A new `/admin/*` API (session cookie auth) — used only by the dashboard.

The SPA's static assets are embedded in the server binary at build time via `include_dir`. The server serves them at `/dashboard/*` with an SPA fallback (unknown paths under `/dashboard/*` return `index.html` so client-side routing works).

```
                                ┌──────────────────────────────────────────────┐
                                │           teramind-sync-server                │
                                │                                                │
 CLI / daemon / MCP             │   /v1/*    DPoP-signed API   (unchanged)      │
 (DPoP-signed) ─────────────────►   /v1/rpc, /v1/ingest, /v1/events, …          │
                                │                                                │
                                │   /admin/*  cookie-auth API   (new)            │
 Browser (admin) ───────────────►   /admin/login, /admin/skills, /admin/events,  │
 (session cookie)               │     /admin/quality, /admin/members, …         │
                                │                                                │
                                │   /dashboard/*  static SPA assets (new)        │
 Browser (admin) ───────────────►   (embedded via include_dir!)                  │
                                │                                                │
                                │   shared internals:                            │
                                │     Postgres                                   │
                                │     Plan L broadcast bus  ──┐                  │
                                │     Plan I/J/M repos        │                  │
                                │     codifier_worker         │                  │
                                │   ┌────────────────────────┐│                  │
                                │   │ event-log writer wraps ││                  │
                                │   │ every bus.send() call  ││                  │
                                │   └────────────────────────┘│                  │
                                │     quality_scheduler ──────┘                  │
                                │     team_event_log + quality_runs tables       │
                                └──────────────────────────────────────────────┘
```

### 3.1 New components

| Layer | Component | Lives at | Responsibility |
|---|---|---|---|
| HTTP | Admin auth middleware | `crates/teramind-sync-server/src/admin_api/auth.rs` | Verifies the `tmd_admin` cookie's HMAC + expiry. Attaches `AdminSession` to requests. |
| HTTP | Admin handlers | `crates/teramind-sync-server/src/admin_api/handlers/*` | One file per view: `activity.rs`, `skills.rs`, `candidates.rs`, `members.rs`, `quality.rs`, `health.rs`, `session.rs`. |
| HTTP | Static asset serving | `crates/teramind-sync-server/src/dashboard_assets.rs` | Reads embedded `include_dir!` bundle; serves with content-type guessing; falls back to `index.html`. |
| Background | Quality scheduler | `crates/teramind-sync-server/src/quality_scheduler.rs` | Cron-driven loop that runs `teramind-search-eval` as a subprocess and persists results. |
| Background | Event-log writer | shim around the existing broadcast site | Fire-and-forget DB insert next to every `bus.send()`. |
| Background | Event-log pruner | `crates/teramind-sync-server/src/event_log_pruner.rs` | Periodic delete of rows older than retention. |
| Storage | Migration | `crates/teramind-db/migrations/20260520000001_dashboard.sql` | `team_event_log` + `quality_runs` tables. |
| Frontend | SPA | `dashboard/` at the repo root | React 18 + TypeScript + TanStack Query/Router + Tailwind + Recharts + Vite. |

### 3.2 What's NOT new

- No new top-level Rust crate. All server-side code lands inside `teramind-sync-server`.
- No new IPC variants. The dashboard speaks to its own admin endpoints; the existing `/v1/rpc` and `Request`/`Response` enums (Plan K) are not touched.
- No new auth infrastructure for non-browser clients. CLI / daemon / MCP keep DPoP.

---

## 4. Auth + session cookies

### 4.1 Password hashing

A new sync-server CLI subcommand prompts for a password and prints the hash + a fresh session secret. The operator pastes both into `config.toml`.

```
$ teramind-sync-server admin-password
Enter new admin password: ********
Confirm: ********
admin_password_hash = "$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>"
admin_session_secret = "<32 random hex chars>"
```

Argon2 parameters (OWASP 2024 minimum): `m=19MiB, t=2, p=1`. Crate: `argon2` 0.5. The session secret is a 32-byte hex string used as the HMAC key for cookie signing.

### 4.2 Config

Operator-managed in `/etc/teramind-sync-server/config.toml`:

```toml
[admin]
admin_password_hash      = "$argon2id$v=19$..."
admin_session_secret     = "..."          # 32-byte hex, HMAC key
admin_session_ttl_hours  = 12             # optional; default 12
event_log_retention_days = 90             # optional; default 90
```

When `[admin]` is absent, the dashboard is **disabled**: `/admin/*` and `/dashboard/*` both return 404. Enabling the dashboard is explicit opt-in.

### 4.3 Login

`POST /admin/login` body `{ "password": "..." }`:

- Verifies the password against `admin_password_hash` using argon2id.
- On success: generates a 16-byte random `jti`, computes `expires_at = now + admin_session_ttl_hours`, encodes the cookie token as:
  ```
  payload = base64url(jti || expires_at_unix_be64)
  signature = base64url(hmac_sha256(admin_session_secret, payload))
  token = payload + "." + signature
  ```
  Sets `Set-Cookie: tmd_admin=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=43200`.
- On failure: 401, with rate-limit accounting (below).

Response body: `{ "logged_in": true, "expires_at": "<ISO-8601>" }`.

### 4.4 Auth middleware

Wraps every `/admin/*` route except `/admin/login`, `/admin/logout`, `/admin/version`. On each request:

1. Read `tmd_admin` cookie. Missing → 401.
2. Split on `.`; HMAC-verify the signature against the payload using `admin_session_secret`. Mismatch → 401.
3. Parse the payload; check `expires_at >= now`. Expired → 401.
4. Attach `AdminSession { expires_at, jti }` to the request and continue.

The server holds **no per-session state** — the cookie is self-validating via HMAC. Rotating `admin_session_secret` instantly invalidates every outstanding cookie.

### 4.5 Logout

`POST /admin/logout` sets `Set-Cookie: tmd_admin=; Max-Age=0; Path=/`. No server-side action.

### 4.6 Rate limiting

In-memory `LRU<IpAddr, (failures, last_attempt)>` with 200 entries. After 5 failed logins from one IP within 60s, all login attempts from that IP return 429 for the next 5 minutes. Resets on a successful login. No DB persistence — server restart clears the throttle.

### 4.7 SPA-side auth state

The SPA mounts a `useAuth` hook that fires `GET /admin/me` on app load. 200 → render; 401 → navigate to `/dashboard/login?redirect=<original-path>`. The login form POSTs to `/admin/login`; on 200 it navigates to the original path. A logout button posts to `/admin/logout` and resets the in-app auth state.

---

## 5. Admin API endpoints

All routes under `/admin/*`. All responses JSON. Auth required for everything except `login`, `logout`, `version`.

### 5.1 Session + meta

```
POST /admin/login                  { password }                  → 200 + Set-Cookie | 401 | 429
POST /admin/logout                                               → 200 + clear-cookie
GET  /admin/me                                                  → 200 { admin: true, expires_at } | 401
GET  /admin/version                                              → 200 { version: "..." }
```

### 5.2 Activity feed

```
GET /admin/activity?limit=100&before=<ts>&kind=session_ended|skill_saved|wiki_page_ready&user_id=<uuid>
    → 200 { events: [TeamEvent...], next_before: "<ts>" }
GET /admin/events                  (WebSocket; same TeamEvent stream as /v1/events but cookie-auth)
```

The HTTP GET reads from the new `team_event_log` table. The WS endpoint wraps the existing `tokio::broadcast::Sender<TeamEvent>` from Plan L; the only difference vs. `/v1/events` is the auth gate.

### 5.3 Skills catalog

```
GET    /admin/skills?source=all|authored|codified|imported&q=<search>&limit=&offset=
       → 200 { skills: [{ id, name, description, source, applies_to_cwds, source_session_ids, created_at, updated_at }], total }
GET    /admin/skills/<id_or_name>
       → 200 { …full skill including body }
DELETE /admin/skills/<id>                                          → 200
```

### 5.4 Skill candidates (the review UX Plan M deferred)

```
GET   /admin/candidates?status=pending|approved|rejected|all&limit=&offset=
      → 200 { candidates: [Candidate...], total }
GET   /admin/candidates/<id>                                       → 200 { …full Candidate }
POST  /admin/candidates/<id>/approve     { reviewer? }             → 200 { skill_id }
POST  /admin/candidates/<id>/reject      { reviewer?, reason? }    → 200
PATCH /admin/candidates/<id>             { description?, body?, applies_to_cwds? }
      → 200 { … }     (mutates while still `pending`)
```

`approve` writes `status='approved'` + `reviewer` (taken from the session — for v1 always `"admin"`) and then synchronously calls Plan M's `promote::promote_approved_batch(...)` so the new skill is live before the response returns. Idempotent on retry.

### 5.5 Codifier observations (debugging surface)

```
GET /admin/observations?kind=tool_chain|problem_fix|llm_proposal&status=open|synthesized|skipped&min_freq=&limit=
    → 200 { observations: [ObservationRow], total }
GET /admin/observations/<id>                                       → 200 { full row including context_blob }
```

### 5.6 Members + devices

```
GET    /admin/members
       → 200 { users: [{ id, email, display_name, created_at, revoked_at, device_count, last_seen_at }] }
POST   /admin/members/<user_id>/revoke                             → 200
GET    /admin/members/<user_id>/devices
       → 200 { devices: [{ id, name, created_at, last_seen_at, revoked_at }] }
POST   /admin/devices/<device_id>/revoke                           → 200
GET    /admin/invites                                              → 200 { invites: [Invite...] }
POST   /admin/invites                  { email, display_name?, expires_in_days? }
       → 201 { code: "TM-…", invite_id }
POST   /admin/invites/<id>/revoke                                  → 200
```

These are thin shells around Plan I's `crates/teramind-sync-server/src/admin.rs` functions (`invite_create`, `member_revoke_device`, etc.). Created invites' codes return **once**, in the POST body; subsequent GETs only expose the hash. The SPA shows the code in a one-time copy-to-clipboard modal.

### 5.7 Search-quality history

```
GET  /admin/quality?since=<ts>&limit=                              → 200 { runs: [QualityRun...] }
GET  /admin/quality/latest                                         → 200 { run: QualityRun | null }
POST /admin/quality/runs                                           → 201      (manual upload of an eval result)
GET  /admin/quality/config                                         → 200 { enabled, cron, last_run_at, next_run_at }
```

### 5.8 Health

```
GET /admin/health
    → 200 {
        db: "ok" | "degraded",
        broadcast_subscribers: N,
        codifier_backlog: N,
        team_sync: "n/a (server)" | "...",
        quality_scheduler: { enabled, last_run_at, next_run_at },
        ingest: { queue_depth, accepted_24h, dropped_24h },
        uptime_seconds: N,
      }
```

A more verbose, admin-eyes-only sibling of the public `/v1/health`.

### 5.9 Error shape

```json
{ "error": { "code": "candidate_not_found", "message": "no candidate with id ...", "details": {} } }
```

Stable codes: `invalid_password`, `rate_limited`, `unauthorized`, `not_found`, `conflict`, `validation_failed`, `internal`. The SPA renders `error.message` in a toast and uses `error.code` for programmatic flow (e.g. re-route to login on `unauthorized`).

---

## 6. Frontend layout + views

### 6.1 Tech stack

- React 18 + TypeScript.
- Vite for the dev server and production build.
- TanStack Query for data fetching (caching, refetch-on-focus, optimistic mutations).
- TanStack Router for typed routes.
- TailwindCSS for styling.
- Recharts for the quality charts.
- Lucide for icons.
- Vitest for unit tests; Playwright for E2E (server-driven).

No global state manager — TanStack Query's cache handles server state; React's `useState` handles ephemeral UI state. No design system; ~10–15 small primitives on Tailwind.

**Bundle target:** ≤ 250 KB gzipped including all four views. CI asserts the threshold; if it grows, lazy-load the Quality route first (Recharts is the heaviest dep).

### 6.2 Shell

220-px sidebar on desktop, collapses behind a hamburger on mobile.

```
┌─────────────────────────────────────────────────────────┐
│ TERAMIND DASHBOARD                  alice@acme · Logout │
├──────────┬──────────────────────────────────────────────┤
│ Activity │                                              │
│ Skills   │                                              │
│ Members  │            <route content>                   │
│ Quality  │                                              │
│ ─────    │                                              │
│ Health   │                                              │
└──────────┴──────────────────────────────────────────────┘
```

Top bar shows the host the server runs on (since the password-only login has no per-user identity) and a Logout button.

### 6.3 Activity view (`/dashboard/activity`)

Single timeline column.

```
16:32:14  alice@acme       session ended     /openvms-rsync
16:31:02  bob@acme         wiki ready        /openvms-llvm   "ASan port: vfork constraints"
16:28:47  alice@acme       skill saved       vms-autoconf-fork-probe
16:25:11  carol@acme       session ended     /api-gateway
─── 16:20 today ───
15:58:41  alice@acme       skill saved       rust-pr-prep
```

Top of page: filter dropdown (kind, user, project-cwd prefix) and an Auto-refresh toggle. On Auto, the page upgrades from the HTTP-fetched history to a live WebSocket subscription that prepends new events. Each row is clickable into the relevant view (session detail, skill detail, wiki).

### 6.4 Skills view (`/dashboard/skills`)

Two-column layout — left = filterable list, right = detail panel.

```
┌────────────────────┬─────────────────────────────────────┐
│ Skills (143)       │  rust-pr-prep                       │
│ [search ░░░░░]    │  source: codified · seeded from 4   │
│ ━━━━━━━━━━━━━━━━━ │  applies_to: /openvms-*             │
│ ◯ All              │  ─────────────────────────────────  │
│ ◯ Authored (12)    │  description: Build + test + commit │
│ ● Codified (47)    │  ─────────────────────────────────  │
│ ◯ Pending (8)      │  # rust-pr-prep                     │
│ ─────              │                                     │
│ rust-pr-prep       │  ```bash                            │
│ vms-autoconf-fork… │  cargo build && cargo test          │
│ python-pytest-xdi… │  git commit -m "…"                  │
│ docker-compose-up  │  ```                                │
│ react-component-…  │                                     │
└────────────────────┴─────────────────────────────────────┘
```

Filter tabs: `All | Authored | Codified | Pending | Rejected`. The "Pending" tab opens the candidate review UI.

### 6.5 Candidate review (inside the Skills view)

Selecting a pending candidate replaces the right-panel with the review form:

```
Candidate: vms-autoconf-fork-probe        status: PENDING
Seeded from 3 sessions · applies_to: /openvms-*
Generated 4h ago by ollama:qwen3.6:latest (1247 in / 380 out tokens)
─────────────────────────────────────────────────────────
Description:
[editable textarea]
─────────────────────────────────────────────────────────
Body:
[editable Markdown textarea, ~30 lines tall]
─────────────────────────────────────────────────────────
Applies to cwds:
[editable list, one per line]
─────────────────────────────────────────────────────────
▸ View seed sessions (3)        ← expandable
─────────────────────────────────────────────────────────
[Reject]    [Save edits]    [Approve & Promote]
```

- `Save edits` → `PATCH /admin/candidates/<id>` with the dirty fields.
- `Approve & Promote` → `POST /admin/candidates/<id>/approve`; on 200 navigate to the new skill's detail page.
- `Reject` → confirmation dialog with optional reason; `POST /admin/candidates/<id>/reject`.

### 6.6 Members & devices view (`/dashboard/members`)

```
Members (12)                              [+ Issue invite]
────────────────────────────────────────────────────────
email           devices   last seen     status
alice@acme      2         3m ago        active     [view devices ▸]
bob@acme        1         18m ago       active     [view devices ▸]
carol@acme      1         2h ago        active     [view devices ▸]
eve@acme        0         (never)       revoked    [view devices ▸]
────────────────────────────────────────────────────────
Open invites (1)
alice2@acme     expires 2026-05-24                  [revoke]
```

Clicking a member expands inline to show their devices. `+ Issue invite` opens a modal (email, optional display name, expires_in_days slider 1–30); on submit the result is shown in a one-time copy-to-clipboard modal with strong "this is the only time you'll see this code" copy.

### 6.7 Quality view (`/dashboard/quality`)

Three stacked line charts: nDCG@10, MRR, p95 latency. A summary card below shows the latest run's numbers + the model fingerprint.

When the schedule is disabled and no manual runs have been uploaded:

```
No eval history yet.

Run periodic search-quality benchmarks by adding `[quality]` to your config:
  [quality]
  enabled = true
  cron    = "0 2 * * *"   # 02:00 daily

Or upload a one-off result:  POST /admin/quality/runs
```

### 6.8 Health view (`/dashboard/health`)

Text-and-stats — DB status, broadcast subscriber count, codifier backlog, last sync per worker, server uptime. A browser-rendered sibling of the (future) `teramind-sync-server doctor`.

---

## 7. Storage + persistence

### 7.1 Migration `20260520000001_dashboard.sql`

```sql
-- Activity history: every TeamEvent that fires server-side is also persisted.
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

-- Search-quality benchmark history.
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

### 7.2 Event-log writer

Every site that calls `state.bus.send(TeamEvent::…)` (Plan L's ingest handler, Plan L's `SkillSaved` in `rpc_dispatch`, future summarizer writes) also inserts a row into `team_event_log`. The writes are sequential — bus first, then a fire-and-forget DB insert spawned on a tokio task. Failure to persist does not roll back the broadcast.

### 7.3 Event-log pruner

`crates/teramind-sync-server/src/event_log_pruner.rs` spawns a `tokio::time::interval` task that runs every 6 hours:

```sql
DELETE FROM team_event_log WHERE ts < now() - $1 * INTERVAL '1 day';
```

`$1` is `event_log_retention_days` from `[admin]` (default 90). Conservative — a busy team accumulates a few million rows over 90 days; that's well within Postgres comfort.

### 7.4 Quality scheduler

New config block (optional; absent = disabled):

```toml
[quality]
enabled    = true
cron       = "0 2 * * *"     # 02:00 UTC daily
baselines  = ["lexical", "semantic"]
eval_binary = "teramind-search-eval"   # default; override path if not on PATH
```

On startup, when `[quality].enabled == true`, the server spawns:

```rust
// crates/teramind-sync-server/src/quality_scheduler.rs
pub fn spawn(pool: DbPool, cfg: QualityConfig) -> tokio::task::JoinHandle<()>
```

Uses the `cron` crate (small dep, ~50 KB) for schedule parsing. Each tick, for each baseline in `cfg.baselines`:

1. `tokio::process::Command::new(&cfg.eval_binary).arg("--baseline").arg(baseline).arg("--json").output().await`
2. Parse stdout as `QualityRunOutput` (shape lives in `teramind-core::quality`).
3. Insert a `quality_runs` row with `source = 'scheduled'`.

Failures (binary missing, non-zero exit, JSON parse fail) log a warning and write a `quality_runs` row with `raw_json = {"error": "..."}` + NaN metrics; the failure surfaces on the Health page.

Single-flight per baseline: if the previous run is still running when the next tick fires, the new tick is skipped and logged.

### 7.5 Manual upload

`POST /admin/quality/runs` accepts the same `QualityRunOutput` JSON. Two use cases:

- The operator runs eval ad-hoc and wants the result on the dashboard.
- CI runs eval and POSTs the result as part of its workflow.

CI uploads use the admin password from a CI secret. v1 doesn't add a separate machine-credential path; v1.1 may.

### 7.6 Eval-binary contract

The existing `teramind-search-eval` binary (Plan F) must gain a small flag addition in this spec's scope: a `--json` flag that emits the metrics as a single JSON object matching `teramind_core::quality::QualityRunOutput`. The binary already computes the metrics; this is a thin serialization layer.

`QualityRunOutput` lives in `teramind-core` so both the eval binary (writer) and the server (reader) share the same definition:

```rust
// crates/teramind-core/src/quality.rs (new)
#[derive(Serialize, Deserialize)]
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
    pub per_class: serde_json::Value,
}
```

---

## 8. Build + embed

### 8.1 Repo layout

```
dashboard/                          <- new top-level directory
├── package.json
├── tsconfig.json
├── vite.config.ts
├── tailwind.config.ts
├── index.html
├── src/
│   ├── main.tsx
│   ├── api/                        <- API clients, error mapping
│   ├── components/                 <- ~15 reusable primitives
│   ├── routes/
│   │   ├── login.tsx
│   │   ├── activity.tsx
│   │   ├── skills.tsx
│   │   ├── candidates.tsx          <- shares state with skills route
│   │   ├── members.tsx
│   │   ├── quality.tsx
│   │   └── health.tsx
│   └── lib/                        <- useAuth, route guard, event-stream hook
├── dist/                           <- build output (gitignored except .gitkeep)
└── tests/                          <- vitest unit tests
```

### 8.2 Build pipeline

```
cd dashboard
npm install
npm run build
```

…produces `dashboard/dist/`: hashed `assets/*.js`, hashed `assets/*.css`, `index.html`.

### 8.3 Embedding

In `crates/teramind-sync-server/src/dashboard_assets.rs`:

```rust
use include_dir::{include_dir, Dir};

static DASHBOARD: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../dashboard/dist");

pub fn lookup(path: &str) -> Option<(&'static [u8], &'static str)> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let file = DASHBOARD.get_file(path)
        .or_else(|| DASHBOARD.get_file("index.html"))?;  // SPA fallback
    let content_type = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js")   => "application/javascript",
        Some("css")  => "text/css",
        Some("svg")  => "image/svg+xml",
        Some("png")  => "image/png",
        Some("ico")  => "image/x-icon",
        _            => "application/octet-stream",
    };
    Some((file.contents(), content_type))
}
```

`include_dir!` with a missing-or-empty `dist/` produces an empty `Dir`, so the handler simply 404s; the server still builds. This lets backend-only contributors build the server without `npm`. A `build.rs` warning surfaces the missing dist:

```rust
// crates/teramind-sync-server/build.rs
fn main() {
    let dist = std::path::Path::new("../../dashboard/dist");
    if !dist.join("index.html").exists() {
        println!("cargo:warning=dashboard/dist not found; server will not serve /dashboard");
    }
    println!("cargo:rerun-if-changed=../../dashboard/dist");
}
```

### 8.4 Route registration

In `crates/teramind-sync-server/src/server.rs::build_router`:

```rust
.route("/dashboard",         get(serve_dashboard_index))
.route("/dashboard/{*path}", get(serve_dashboard_asset))
```

`serve_dashboard_asset` extracts the wildcard, calls `dashboard_assets::lookup`, returns bytes + `Content-Type` (with the SPA-fallback to `index.html` baked in).

### 8.5 Release CI

The release CI matrix (Plan E) gains one step before the Rust build: `cd dashboard && npm ci && npm run build`. The resulting `dist/` is then embedded by `cargo build --release -p teramind-sync-server`. Every released binary ships the dashboard bundled.

### 8.6 Local dev

- Backend devs: `cargo build -p teramind-sync-server` (warns, 404s on `/dashboard/*`).
- Frontend devs: `cd dashboard && npm install && npm run dev` runs Vite on `:5173` with a proxy to the backend at `:8443`. Backend serves `/admin/*`; frontend dev server serves `/dashboard/*`. No embedding during development.

---

## 9. Testing strategy

### 9.1 L1 — Unit

**Rust:**
- Argon2 verify: correct password → `true`; wrong → `false`; malformed hash → typed error.
- Session cookie codec round-trips; expired token rejects; tampered HMAC rejects.
- Cron parser wrapper: `0 2 * * *`, `*/15 * * * *`, `@daily` compute the right next-fire time.
- `QualityRunOutput` JSON round-trips; missing fields error cleanly; extras ignored.
- Asset content-type mapping.

**TypeScript (vitest):**
- `useAuth` hook: 200 from `/admin/me` → `authenticated: true`; 401 → redirect.
- API client error mapping: structured error JSON → typed `DashboardError` with `.code`, `.message`, `.details`.
- Activity event reducer: prepends new events; evicts the oldest when state exceeds 200.

### 9.2 L2 — Component (real PG, per-endpoint)

- `/admin/login`: 200 + cookie on correct password; 401 on wrong; 429 after 5 failures from one IP; reset on success.
- `/admin/me`: 200 with valid cookie; 401 missing / expired / tampered.
- `/admin/skills?source=codified`: filters correctly.
- `/admin/candidates/<id>/approve`: writes `status='approved'` AND synchronously promotes; the `skills` row exists with `source='codified'` before the response returns.
- `PATCH /admin/candidates/<id>`: updates only specified fields; `status` stays `pending`.
- `/admin/candidates/<id>/reject`: writes `status='rejected'` + `reviewer`. A subsequent approve returns 409.
- `POST /admin/invites`: returns the code exactly once; subsequent GETs never include the code.
- `/admin/events` WS: subscribes with valid cookie; 401 missing; receives an event within 500ms of `state.bus.send(...)`.
- `POST /admin/quality/runs`: validates the JSON (NaN rejected, out-of-range percentiles rejected); inserts a row.
- Event-log writer: every `bus.send()` site also inserts a `team_event_log` row (verified by sending three event kinds and counting by kind).

### 9.3 L3 — Integration (full server + headless browser)

Built `dashboard/dist/` is embedded for the test run.

- `GET /dashboard/` returns `index.html` 200.
- `GET /dashboard/assets/<hash>.js` returns 200 with `Content-Type: application/javascript`.
- `GET /dashboard/skills/123/edit` (a client-side route) returns `index.html` 200 — SPA fallback works.
- Playwright: launch a headless browser, navigate to `/dashboard/`, log in, verify each of the four top-level routes loads + renders its sentinel text.
- Playwright: approve a pending candidate; verify the row moves to the codified list within 2 seconds.
- Playwright: live update — `state.bus.send()` from a test helper, assert the event appears in the Activity feed within 500ms.

### 9.4 L4 — E2E (nightly, real Claude Code lab)

Reuses the two-developer setup from Plans I/J/K. After a scripted Claude session ends, the test:

1. Logs into the dashboard.
2. Verifies the session appears in the Activity feed.
3. Navigates to the resulting wiki page.
4. If the codifier fires a candidate (deterministic with `min_observation_frequency=2` for the test), navigates to it, approves it, and verifies it lands in the Skills tab.

### 9.5 L5 — Quality history flow

- Point the eval binary at a mock that emits a known JSON output.
- Configure `cron = "* * * * *"` (every minute) in a test config.
- Run for 90s; assert 1–2 rows landed in `quality_runs`.
- Assert `GET /admin/quality` returns those rows.
- Playwright: assert the Quality chart renders the expected y-values via a DOM assertion.

### 9.6 Security tests

- Brute-force `POST /admin/login` with 1000 wrong passwords from one IP → all rate-limited after the 5th. Switch source IP → throttle starts fresh.
- Cookie with the HMAC byte at index 12 flipped → 401.
- Cookie with `expires_at` 1 second in the past → 401.
- Cross-origin POST from `evil.example.com` to `/admin/me` with a valid cookie → CORS rejects the response (the `SameSite=Strict` already prevents the browser sending it; this verifies defense in depth).
- CSP header on `/dashboard/*` responses: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' wss://<host>; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'`. Inline `<style>` is allowed because Tailwind emits some inline `style` attributes for dynamic values.

### 9.7 Performance budgets

| Path | Budget |
|---|---|
| `POST /admin/login` (argon2-bound) | p99 < 100 ms |
| `GET /admin/activity?limit=100` against 100 k rows | p99 < 200 ms |
| `GET /admin/skills?source=codified` (1 k rows) | p99 < 150 ms |
| WS publish → browser receives | p99 < 200 ms intra-region |
| SPA: cold load → first paint | < 2 s on fast connection |
| SPA bundle gzipped | ≤ 250 KB (CI gated) |

---

## 10. Rollout, dependencies, risks

### 10.1 Phases

1. **v1 (this spec)** — All four views, admin-password auth, scheduled eval (optional), embedded SPA, candidate review with edit/approve/reject.
2. **v1.1** — SSO/OAuth as an alternative auth path; 2FA for the password login; per-baseline custom eval schedules.
3. **v1.2** — Multi-tenancy (when the sync server supports multiple teams). Audit log of who approved which candidate (UI surface; the `reviewer` column captures the data today). Read-only viewer accounts.
4. **v2+** — Embeddable widgets; ingest-rate / error-budget dashboards; eval-result alerting (Slack/email when nDCG@10 regresses).

### 10.2 Dependencies

Builds on Plans A–M. Specifically: Plan I (admin module), Plan L (broadcast bus, `/v1/events`), Plan M (skill candidates), Plan F (eval binary). No blockers in flight; the dashboard plan can ship after Plan M lands.

### 10.3 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Brute-force the admin password | Medium | Argon2id + per-IP rate limit; failure surfaced via Health page so operators see attempts. SSO in v1.1. |
| SPA bundle bloat over time | Medium | Hard 250 KB CI gate; lazy-load Quality if it exceeds. |
| `npm` toolchain breaks CI for backend contributors | Low | Backend devs don't need `npm`; missing dist warns + 404s, doesn't block. |
| Embedded dist gets stale during dev | Medium | Dev workflow uses Vite + proxy. Documented in CONTRIBUTING. |
| Scheduled eval runs eat resources | Low | Default schedule is daily; configurable; single-flight per baseline. |
| Manual quality upload lets a malicious admin spoof history | Accepted | Admin already has every other privilege; spoofing eval numbers is not a meaningful new attack vector. |
| Browser holds session indefinitely if user closes tab without logout | Low | 12-hour TTL. Logout clears immediately. |
| Cron 5-field vs 6-field parsing surprises | Low | Unit-test the common patterns; document. |
| Approve UI conflicts with concurrent SQL UPDATEs | Low | The promote loop is idempotent under `ON CONFLICT (name) DO UPDATE`; the API uses synchronous promotion to give immediate UI feedback. |
| Event-log writer fails behind a slow DB | Low | Fire-and-forget; bus subscribers already got the event. The pruner ensures the table stays bounded. |

### 10.4 Out of scope

Listed in §2.2. The most important deferrals: **SSO/OAuth** (v1.1) and **viewer role** (v2+). v1 is strictly admin-only password auth.

---

## 11. Glossary

- **Admin** — The operator who self-hosts the sync server. v1's only dashboard user.
- **Session cookie** — `tmd_admin`; self-validating via HMAC against `admin_session_secret`. No server-side session table.
- **Candidate** — A skill candidate (Plan M). Pre-approval row in `skill_candidates`. The dashboard's Skills view exposes the v1.1-deferred review UX (edit / approve / reject) here in v1.
- **Promotion** — Plan M's transactional INSERT into `skills` + UPDATE of the candidate to `'promoted'`. The dashboard's approve handler short-circuits to a synchronous call so the new skill is visible immediately.
- **TeamEvent** — Plan L's event variants (`SessionEnded`, `WikiPageReady`, `SkillSaved`). Now also persisted to `team_event_log`.
- **Quality run** — One execution of `teramind-search-eval` against the L5 corpus. Persisted to `quality_runs` either by the scheduler, by manual upload, or by CI.
- **applies_to_cwds** — Plan M's per-skill scope. Used by the SessionStart digest filter and exposed in the Skills detail view.

---

*End of spec.*
