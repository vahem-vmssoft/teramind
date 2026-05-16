# Teramind Sync Server (Plan I) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the central `teramind-sync-server` binary — invite-based device redemption, DPoP-signed request authentication, and a `POST /v1/ingest` endpoint that lands captured events from remote daemons into a server-side Postgres with `(user_id, device_id)` annotation. This is the foundation for team mode; the local forwarder, MCP proxy, and live propagation arrive in Plans J, K, L.

**Architecture:** New bin crate `teramind-sync-server` builds an axum-based HTTPS server against operator-provided Postgres. Three new repos (`UserRepo`, `DeviceRepo`, `InviteRepo`) plus an additive migration. Tower middleware verifies `Authorization: Bearer` + `X-Teramind-Proof` (Ed25519 DPoP). Admin CLI subcommands (`serve`, `migrate`, `invite create|list|revoke`, `member list|revoke-device|revoke-user`). The daemon's `route()` event-dispatch function is refactored into a reusable `RouteDeps`-parameterized fn so the server can reuse the exact same handler set.

**Tech Stack:** Rust stable (workspace pin 1.93.0). New workspace deps: `axum` 0.7, `tower` 0.5, `tower-http` 0.6 (trace, limit), `ed25519-dalek` 2 (pure-Rust Ed25519), `base32` 0.5 (token format), `rustls-pemfile` 2 + `tokio-rustls` 0.26 (TLS termination), `parking_lot` 0.12 (replay-cache mutex). Reuses `teramind-db`, `teramind-core`, and `teramindd::services::ingest::route` (extracted in §15).

---

## Spec coverage

This plan implements §1–§5 and §11–§12 of `docs/superpowers/specs/2026-05-17-teramind-team-sync-design.md` end-to-end, and lays foundations for §6 (forwarder), §7 (MCP proxy), §8 (live events). Coverage matrix at the bottom.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramind-sync-server/Cargo.toml` | Bin crate manifest |
| `crates/teramind-sync-server/src/main.rs` | clap CLI entry: `serve`, `migrate`, `invite …`, `member …`, `version` |
| `crates/teramind-sync-server/src/lib.rs` | Module registry; re-exports for integration tests |
| `crates/teramind-sync-server/src/config.rs` | `ServerConfig` TOML loader |
| `crates/teramind-sync-server/src/server.rs` | axum `Router` + listener + graceful shutdown |
| `crates/teramind-sync-server/src/state.rs` | `AppState` (pool, repos, replay-cache, broadcast channel placeholder) |
| `crates/teramind-sync-server/src/auth.rs` | Tower middleware: extract bearer + proof, attach `AuthContext` |
| `crates/teramind-sync-server/src/proof.rs` | DPoP claim struct + Ed25519 sign/verify + replay cache |
| `crates/teramind-sync-server/src/invite.rs` | Invite-code generation/parsing/hashing |
| `crates/teramind-sync-server/src/token.rs` | Bearer-token generation/parsing/hashing |
| `crates/teramind-sync-server/src/handlers/mod.rs` | Module registry |
| `crates/teramind-sync-server/src/handlers/health.rs` | `GET /v1/health`, `GET /v1/version` |
| `crates/teramind-sync-server/src/handlers/redeem.rs` | `POST /v1/auth/redeem` |
| `crates/teramind-sync-server/src/handlers/ingest.rs` | `POST /v1/ingest` |
| `crates/teramind-sync-server/src/admin.rs` | Subcommand bodies (invite/member) |
| `crates/teramind-sync-server/src/tls.rs` | rustls config from PEM files |
| `crates/teramind-db/migrations/20260517000001_team_mode.sql` | Tables + ALTERs |
| `crates/teramind-db/src/repos/user.rs` | `UserRepo` |
| `crates/teramind-db/src/repos/device.rs` | `DeviceRepo` |
| `crates/teramind-db/src/repos/invite.rs` | `InviteRepo` |
| `crates/teramind-sync-server/tests/team_harness.rs` | Multi-step integration test against a spawned server |
| `crates/teramind-sync-server/tests/auth_flow.rs` | Redeem + DPoP-protected request happy path |
| `crates/teramind-sync-server/tests/ingest_endpoint.rs` | Ingest annotation + idempotency |
| `docker/sync-server/Dockerfile` | Minimal scratch-runtime image |
| `docker/sync-server/docker-compose.yml` | Server + Postgres for quick start |
| `docs/runbooks/sync-server-deploy.md` | Operator-facing setup runbook |

**Modified files:**

- `Cargo.toml` (workspace) — register the new crate and add `axum`, `tower`, `tower-http`, `ed25519-dalek`, `base32`, `rustls-pemfile`, `tokio-rustls`, `parking_lot`.
- `crates/teramind-db/src/repos/mod.rs` — register `user`, `device`, `invite` modules + re-exports.
- `crates/teramindd/src/services/ingest.rs` — extract `route(d, env)` → `route_with_deps(deps: &RouteDeps, env, auth: Option<IngestAuth>)`. The daemon path passes `auth = None`; the server constructs `IngestAuth { user_id, device_id }` from its axum `AuthContext`.
- `crates/teramindd/src/lib.rs` — re-export `services::ingest::{RouteDeps, route_with_deps}` for the server.

---

## Section 0 — Pre-flight

### Task 0.1: Create the working branch

**Files:** none (git state only)

- [ ] **Step 1: Cut the branch from a green main**

Run:
```bash
git fetch origin
git checkout main
git pull --ff-only
cargo build --workspace
cargo test --workspace --no-run
git checkout -b feat/teramind-sync-server
```

Expected:
- `cargo build --workspace` succeeds with zero warnings on Plan H state.
- `cargo test --workspace --no-run` builds all test binaries.
- HEAD is now `feat/teramind-sync-server`.

If `cargo build` is dirty, stop and surface the failure — the plan assumes a clean Plan-H baseline.

### Task 0.2: Confirm the spec is committed

- [ ] **Step 1: Verify spec presence**

Run: `git log -1 --pretty='%h %s' -- docs/superpowers/specs/2026-05-17-teramind-team-sync-design.md`

Expected: a commit `docs(spec): team sync server (Teramind #4)` exists. If not, stop — the spec must precede the plan.

---

## Section 1 — Workspace + crate skeleton

The first commit just registers the new crate, prints a version banner, and builds. Everything else stacks on this.

### Task 1.1: Register the crate in the workspace

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add the member + new workspace deps**

Edit the workspace root `Cargo.toml`. Append `"crates/teramind-sync-server"` to `members`, and add these to `[workspace.dependencies]` (keeping alphabetical-ish ordering already in the file):

```toml
axum            = { version = "0.7", default-features = false, features = ["http1", "http2", "json", "tokio", "matched-path"] }
axum-server     = { version = "0.7", default-features = false, features = ["tls-rustls"] }
base32          = "0.5"
ed25519-dalek   = { version = "2", default-features = false, features = ["std", "rand_core"] }
parking_lot     = "0.12"
rustls-pemfile  = "2"
tokio-rustls    = "0.26"
tower           = { version = "0.5", default-features = false, features = ["util"] }
tower-http      = { version = "0.6", default-features = false, features = ["limit", "trace"] }
```

- [ ] **Step 2: Verify the workspace still parses**

Run: `cargo metadata --format-version=1 --no-deps > /dev/null`

Expected: exit 0. (No new crate yet — this just confirms the manifest edits are well-formed.)

### Task 1.2: Crate manifest

**Files:**
- Create: `crates/teramind-sync-server/Cargo.toml`

- [ ] **Step 1: Write the manifest**

```toml
[package]
name = "teramind-sync-server"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lib]
name = "teramind_sync_server"
path = "src/lib.rs"

[[bin]]
name = "teramind-sync-server"
path = "src/main.rs"

[dependencies]
teramind-core = { path = "../teramind-core" }
teramind-db   = { path = "../teramind-db" }
teramind-ipc  = { path = "../teramind-ipc" }
teramindd     = { path = "../teramindd" }

anyhow         = { workspace = true }
async-trait    = { workspace = true }
axum           = { workspace = true }
axum-server    = { workspace = true }
base32         = { workspace = true }
clap           = { workspace = true }
ed25519-dalek  = { workspace = true }
hex            = { workspace = true }
parking_lot    = { workspace = true }
rand           = { workspace = true }
rustls-pemfile = { workspace = true }
serde          = { workspace = true }
serde_json     = { workspace = true }
sha2           = { workspace = true }
sqlx           = { workspace = true }
thiserror      = { workspace = true }
time           = { workspace = true }
tokio          = { workspace = true }
tokio-rustls   = { workspace = true }
toml           = { workspace = true }
tower          = { workspace = true }
tower-http     = { workspace = true }
tracing        = { workspace = true }
tracing-appender   = { workspace = true }
tracing-subscriber = { workspace = true }
uuid           = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
proptest = { workspace = true }
reqwest  = { workspace = true }
```

### Task 1.3: Library + binary scaffold

**Files:**
- Create: `crates/teramind-sync-server/src/lib.rs`
- Create: `crates/teramind-sync-server/src/main.rs`

- [ ] **Step 1: Write `lib.rs`**

```rust
//! Teramind central sync server. See docs/superpowers/specs/2026-05-17-teramind-team-sync-design.md.

pub mod admin;
pub mod auth;
pub mod config;
pub mod handlers;
pub mod invite;
pub mod proof;
pub mod server;
pub mod state;
pub mod tls;
pub mod token;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

(`admin`, `auth`, `config`, `handlers`, `invite`, `proof`, `server`, `state`, `tls`, `token` are created in later tasks as empty `pub mod` files first so this compiles; see Step 2.)

- [ ] **Step 2: Stub the modules so `lib.rs` compiles**

For each module listed in `lib.rs`, create the file with this single-line stub:

```rust
//! Placeholder; populated in a later task.
```

Files to create (each with the line above):
- `crates/teramind-sync-server/src/admin.rs`
- `crates/teramind-sync-server/src/auth.rs`
- `crates/teramind-sync-server/src/config.rs`
- `crates/teramind-sync-server/src/handlers.rs` (wait — using a `handlers/` directory; create `crates/teramind-sync-server/src/handlers/mod.rs` instead)
- `crates/teramind-sync-server/src/invite.rs`
- `crates/teramind-sync-server/src/proof.rs`
- `crates/teramind-sync-server/src/server.rs`
- `crates/teramind-sync-server/src/state.rs`
- `crates/teramind-sync-server/src/tls.rs`
- `crates/teramind-sync-server/src/token.rs`
- `crates/teramind-sync-server/src/handlers/mod.rs`

- [ ] **Step 3: Write `main.rs`**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "teramind-sync-server", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print version and exit.
    Version,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Version => {
            println!("teramind-sync-server {}", teramind_sync_server::VERSION);
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Build the new crate**

Run: `cargo build -p teramind-sync-server`

Expected: success. One compiled binary at `target/debug/teramind-sync-server`.

- [ ] **Step 5: Smoke the binary**

Run: `./target/debug/teramind-sync-server version`

Expected output:
```
teramind-sync-server 0.1.0
```

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/teramind-sync-server
git commit -m "feat(sync-server): crate skeleton + version subcommand"
```

---

## Section 2 — Schema migration

The migration is **server-side only**. Local-first installs (Plans A–H) never apply it. We accomplish this by keeping the SQL file in the shared `crates/teramind-db/migrations/` directory; the server build runs all migrations, the local daemon also runs them. That works because the migration is purely additive — it adds tables and nullable columns, never breaking the local-first schema.

### Task 2.1: Write the migration

**Files:**
- Create: `crates/teramind-db/migrations/20260517000001_team_mode.sql`

- [ ] **Step 1: Write the SQL**

```sql
-- Team-mode tables and additive columns on sessions/skills.
-- Additive only; safe to apply to local-first installs.

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
  token_hash   bytea NOT NULL UNIQUE,
  public_key   bytea NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz,
  revoked_at   timestamptz
);
CREATE INDEX devices_user      ON devices (user_id);
CREATE INDEX devices_last_seen ON devices (last_seen_at DESC);

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
CREATE INDEX invites_email   ON invites (invited_email);
CREATE INDEX invites_expires ON invites (expires_at) WHERE redeemed_at IS NULL;

ALTER TABLE sessions ADD COLUMN user_id   uuid REFERENCES users(id);
ALTER TABLE sessions ADD COLUMN device_id uuid REFERENCES devices(id);
ALTER TABLE skills   ADD COLUMN user_id   uuid REFERENCES users(id);
ALTER TABLE skills   ADD COLUMN device_id uuid REFERENCES devices(id);

CREATE INDEX sessions_user        ON sessions (user_id);
CREATE INDEX sessions_user_recent ON sessions (user_id, started_at DESC);
CREATE INDEX skills_user          ON skills (user_id);
```

- [ ] **Step 2: Write a verification test (under teramind-db)**

**Files:**
- Create: `crates/teramind-db/tests/team_mode_migration.rs`

```rust
//! Verifies the team-mode migration applies cleanly on a fresh PG.

use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn team_mode_migration_creates_tables_and_columns() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    // Tables exist.
    for t in ["users", "devices", "invites"] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)"
        ).bind(t).fetch_one(pool.pg()).await?;
        assert!(exists, "table `{t}` should exist after migration");
    }

    // Additive columns are present on sessions + skills.
    for (table, col) in [
        ("sessions", "user_id"), ("sessions", "device_id"),
        ("skills", "user_id"),   ("skills", "device_id"),
    ] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_name = $1 AND column_name = $2)"
        ).bind(table).bind(col).fetch_one(pool.pg()).await?;
        assert!(exists, "{table}.{col} should exist after migration");
    }

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-db --test team_mode_migration -- --nocapture`

Expected: PASS. (Takes ~10s for embedded PG warmup.)

- [ ] **Step 4: Run the rest of the db tests to confirm no regression**

Run: `cargo test -p teramind-db`

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/migrations/20260517000001_team_mode.sql \
        crates/teramind-db/tests/team_mode_migration.rs
git commit -m "feat(db): add team-mode migration (users/devices/invites + ALTERs)"
```

---

## Section 3 — UserRepo

### Task 3.1: Write the failing test

**Files:**
- Create: `crates/teramind-db/tests/user_repo.rs`

- [ ] **Step 1: Write the test**

```rust
use teramind_db::repos::UserRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

async fn fresh_pool() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upsert_creates_then_returns_existing() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());

    let a = users.upsert_by_email("alice@acme.dev", Some("Alice K.")).await?;
    let b = users.upsert_by_email("alice@acme.dev", Some("Alice K.")).await?;
    assert_eq!(a.id, b.id, "upsert must be idempotent by email");
    assert_eq!(a.email, "alice@acme.dev");

    let none = users.get_by_id(a.id).await?;
    assert!(none.is_some(), "get_by_id should round-trip");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_sets_revoked_at_and_get_filters() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());

    let u = users.upsert_by_email("bob@acme.dev", None).await?;
    users.revoke(u.id).await?;
    let active = users.get_active(u.id).await?;
    assert!(active.is_none(), "revoked user must not appear via get_active");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run the test, watch it fail**

Run: `cargo test -p teramind-db --test user_repo -- --nocapture`

Expected: FAIL with `unresolved import teramind_db::repos::UserRepo`.

### Task 3.2: Implement UserRepo

**Files:**
- Create: `crates/teramind-db/src/repos/user.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Write the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::UserId;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct UserRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: UserId,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

impl UserRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn upsert_by_email(&self, email: &str, display_name: Option<&str>) -> Result<User> {
        let row: (Uuid, String, Option<String>, OffsetDateTime, Option<OffsetDateTime>) =
            sqlx::query_as(
                r#"
                INSERT INTO users (email, display_name)
                VALUES ($1, $2)
                ON CONFLICT (email) DO UPDATE SET display_name = COALESCE(EXCLUDED.display_name, users.display_name)
                RETURNING id, email, display_name, created_at, revoked_at
                "#)
            .bind(email).bind(display_name)
            .fetch_one(self.pool.pg()).await?;
        Ok(User { id: UserId(row.0), email: row.1, display_name: row.2,
                  created_at: row.3, revoked_at: row.4 })
    }

    pub async fn get_by_id(&self, id: UserId) -> Result<Option<User>> {
        let row: Option<(Uuid, String, Option<String>, OffsetDateTime, Option<OffsetDateTime>)> =
            sqlx::query_as(
                "SELECT id, email, display_name, created_at, revoked_at FROM users WHERE id = $1")
            .bind(id.0).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|r| User { id: UserId(r.0), email: r.1, display_name: r.2,
                              created_at: r.3, revoked_at: r.4 }))
    }

    pub async fn get_active(&self, id: UserId) -> Result<Option<User>> {
        Ok(self.get_by_id(id).await?.filter(|u| u.revoked_at.is_none()))
    }

    pub async fn revoke(&self, id: UserId) -> Result<()> {
        sqlx::query("UPDATE users SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<User>> {
        let rows: Vec<(Uuid, String, Option<String>, OffsetDateTime, Option<OffsetDateTime>)> =
            sqlx::query_as(
                "SELECT id, email, display_name, created_at, revoked_at FROM users ORDER BY email")
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter()
            .map(|r| User { id: UserId(r.0), email: r.1, display_name: r.2,
                            created_at: r.3, revoked_at: r.4 })
            .collect())
    }
}
```

- [ ] **Step 2: Register the module + re-export**

Edit `crates/teramind-db/src/repos/mod.rs`. Add `pub mod user;` to the module list and `pub use user::{User, UserRepo};` to the re-exports.

- [ ] **Step 3: Add `UserId` to teramind-core**

Edit `crates/teramind-core/src/ids.rs`. Add a line:

```rust
id_newtype!(UserId);
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p teramind-db --test user_repo -- --nocapture`

Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/ids.rs \
        crates/teramind-db/src/repos/user.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/tests/user_repo.rs
git commit -m "feat(db): UserRepo + UserId newtype"
```

---

## Section 4 — DeviceRepo

### Task 4.1: Write the failing test

**Files:**
- Create: `crates/teramind-db/tests/device_repo.rs`

- [ ] **Step 1: Write the test**

```rust
use teramind_db::repos::{DeviceRepo, UserRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

async fn fresh_pool() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_and_get_by_token_hash_round_trips() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());

    let u = users.upsert_by_email("alice@acme.dev", None).await?;
    let token_hash = vec![0xAAu8; 32];
    let public_key = vec![0xBBu8; 32];
    let d = devices.insert(u.id, "alice-macbook", &token_hash, &public_key).await?;
    let by_hash = devices.get_active_by_token_hash(&token_hash).await?
        .expect("device must be findable by token hash");
    assert_eq!(by_hash.id, d.id);
    assert_eq!(by_hash.public_key, public_key);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_excludes_from_active_lookup() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());

    let u = users.upsert_by_email("bob@acme.dev", None).await?;
    let th = vec![0x01u8; 32]; let pk = vec![0x02u8; 32];
    let d = devices.insert(u.id, "bob-laptop", &th, &pk).await?;
    devices.revoke(d.id).await?;
    let active = devices.get_active_by_token_hash(&th).await?;
    assert!(active.is_none(), "revoked device must not appear via get_active_by_token_hash");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn touch_last_seen_advances() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());

    let u = users.upsert_by_email("carol@acme.dev", None).await?;
    let th = vec![0x03u8; 32]; let pk = vec![0x04u8; 32];
    let d = devices.insert(u.id, "carol-pc", &th, &pk).await?;
    let before = devices.get_active_by_token_hash(&th).await?.unwrap().last_seen_at;
    assert!(before.is_none(), "fresh device has null last_seen_at");
    devices.touch_last_seen(d.id).await?;
    let after = devices.get_active_by_token_hash(&th).await?.unwrap().last_seen_at;
    assert!(after.is_some(), "touch_last_seen must populate last_seen_at");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run, watch it fail**

Run: `cargo test -p teramind-db --test device_repo`

Expected: FAIL — `DeviceRepo` not found.

### Task 4.2: Implement DeviceRepo

**Files:**
- Create: `crates/teramind-db/src/repos/device.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`
- Modify: `crates/teramind-core/src/ids.rs`

- [ ] **Step 1: Add DeviceId**

In `crates/teramind-core/src/ids.rs`, add: `id_newtype!(DeviceId);`

- [ ] **Step 2: Write the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{DeviceId, UserId};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct DeviceRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub user_id: UserId,
    pub name: String,
    pub public_key: Vec<u8>,
    pub last_seen_at: Option<OffsetDateTime>,
}

impl DeviceRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn insert(
        &self,
        user_id: UserId,
        name: &str,
        token_hash: &[u8],
        public_key: &[u8],
    ) -> Result<Device> {
        let row: (Uuid, Uuid, String, Vec<u8>, Option<OffsetDateTime>) = sqlx::query_as(
            r#"
            INSERT INTO devices (user_id, name, token_hash, public_key)
            VALUES ($1, $2, $3, $4)
            RETURNING id, user_id, name, public_key, last_seen_at
            "#)
            .bind(user_id.0).bind(name).bind(token_hash).bind(public_key)
            .fetch_one(self.pool.pg()).await?;
        Ok(Device { id: DeviceId(row.0), user_id: UserId(row.1),
                    name: row.2, public_key: row.3, last_seen_at: row.4 })
    }

    pub async fn get_active_by_token_hash(&self, token_hash: &[u8]) -> Result<Option<Device>> {
        let row: Option<(Uuid, Uuid, String, Vec<u8>, Option<OffsetDateTime>)> = sqlx::query_as(
            r#"
            SELECT d.id, d.user_id, d.name, d.public_key, d.last_seen_at
            FROM   devices d
            JOIN   users u ON u.id = d.user_id
            WHERE  d.token_hash = $1
              AND  d.revoked_at IS NULL
              AND  u.revoked_at IS NULL
            "#)
            .bind(token_hash).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|r| Device { id: DeviceId(r.0), user_id: UserId(r.1),
                                name: r.2, public_key: r.3, last_seen_at: r.4 }))
    }

    pub async fn touch_last_seen(&self, id: DeviceId) -> Result<()> {
        sqlx::query("UPDATE devices SET last_seen_at = now() WHERE id = $1")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn revoke(&self, id: DeviceId) -> Result<()> {
        sqlx::query("UPDATE devices SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn list_for_user(&self, user_id: UserId) -> Result<Vec<Device>> {
        let rows: Vec<(Uuid, Uuid, String, Vec<u8>, Option<OffsetDateTime>)> = sqlx::query_as(
            r#"
            SELECT id, user_id, name, public_key, last_seen_at
            FROM   devices
            WHERE  user_id = $1 AND revoked_at IS NULL
            ORDER BY name
            "#)
            .bind(user_id.0).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter()
            .map(|r| Device { id: DeviceId(r.0), user_id: UserId(r.1),
                              name: r.2, public_key: r.3, last_seen_at: r.4 })
            .collect())
    }
}
```

- [ ] **Step 3: Register + re-export**

In `crates/teramind-db/src/repos/mod.rs`: add `pub mod device;` and `pub use device::{Device, DeviceRepo};`.

- [ ] **Step 4: Run the test**

Run: `cargo test -p teramind-db --test device_repo -- --nocapture`

Expected: all three PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/ids.rs \
        crates/teramind-db/src/repos/device.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/tests/device_repo.rs
git commit -m "feat(db): DeviceRepo + DeviceId newtype"
```

---

## Section 5 — InviteRepo

### Task 5.1: Write the failing test

**Files:**
- Create: `crates/teramind-db/tests/invite_repo.rs`

- [ ] **Step 1: Write the test**

```rust
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use time::{Duration, OffsetDateTime};

async fn fresh_pool() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_and_find_redeemable() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let invites = InviteRepo::new(pool.clone());

    let code_hash = vec![0x10u8; 32];
    let exp = OffsetDateTime::now_utc() + Duration::days(7);
    invites.create(&code_hash, "alice@acme.dev", Some("Alice K."), Some("admin@acme.dev"), exp).await?;
    let found = invites.find_redeemable(&code_hash).await?
        .expect("redeemable invite must be findable");
    assert_eq!(found.invited_email, "alice@acme.dev");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redeemed_invite_is_no_longer_redeemable() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());
    let invites = InviteRepo::new(pool.clone());

    let code_hash = vec![0x20u8; 32];
    let exp = OffsetDateTime::now_utc() + Duration::days(7);
    invites.create(&code_hash, "bob@acme.dev", None, None, exp).await?;
    let u = users.upsert_by_email("bob@acme.dev", None).await?;
    let d = devices.insert(u.id, "bob-laptop", &[0x21u8; 32], &[0x22u8; 32]).await?;
    invites.mark_redeemed(&code_hash, d.id).await?;
    assert!(invites.find_redeemable(&code_hash).await?.is_none());

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_invite_is_not_redeemable() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let invites = InviteRepo::new(pool.clone());
    let code_hash = vec![0x30u8; 32];
    let exp_past = OffsetDateTime::now_utc() - Duration::seconds(1);
    invites.create(&code_hash, "carol@acme.dev", None, None, exp_past).await?;
    assert!(invites.find_redeemable(&code_hash).await?.is_none());
    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run, watch it fail**

Run: `cargo test -p teramind-db --test invite_repo`

Expected: FAIL — `InviteRepo` missing.

### Task 5.2: Implement InviteRepo

**Files:**
- Create: `crates/teramind-db/src/repos/invite.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`
- Modify: `crates/teramind-core/src/ids.rs`

- [ ] **Step 1: Add InviteId**

In `crates/teramind-core/src/ids.rs`: `id_newtype!(InviteId);`

- [ ] **Step 2: Write the repo**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{DeviceId, InviteId};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct InviteRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct Invite {
    pub id: InviteId,
    pub invited_email: String,
    pub display_name: Option<String>,
    pub created_by: Option<String>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub redeemed_at: Option<OffsetDateTime>,
}

impl InviteRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn create(
        &self,
        code_hash: &[u8],
        invited_email: &str,
        display_name: Option<&str>,
        created_by: Option<&str>,
        expires_at: OffsetDateTime,
    ) -> Result<InviteId> {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO invites (code_hash, invited_email, display_name, created_by, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#)
            .bind(code_hash).bind(invited_email).bind(display_name)
            .bind(created_by).bind(expires_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(InviteId(row.0))
    }

    pub async fn find_redeemable(&self, code_hash: &[u8]) -> Result<Option<Invite>> {
        let row: Option<(Uuid, String, Option<String>, Option<String>,
                         OffsetDateTime, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
            r#"
            SELECT id, invited_email, display_name, created_by,
                   created_at, expires_at, redeemed_at
            FROM   invites
            WHERE  code_hash   = $1
              AND  redeemed_at IS NULL
              AND  expires_at  > now()
            "#)
            .bind(code_hash).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|r| Invite {
            id: InviteId(r.0), invited_email: r.1, display_name: r.2,
            created_by: r.3, created_at: r.4, expires_at: r.5, redeemed_at: r.6,
        }))
    }

    /// Marks the invite redeemed. Returns rows_affected — 1 on success, 0 on
    /// race (someone else redeemed first). Caller treats 0 as a 409.
    pub async fn mark_redeemed(&self, code_hash: &[u8], device_id: DeviceId) -> Result<u64> {
        let r = sqlx::query(
            r#"
            UPDATE invites
            SET    redeemed_at = now(), redeemed_device = $2
            WHERE  code_hash = $1 AND redeemed_at IS NULL AND expires_at > now()
            "#)
            .bind(code_hash).bind(device_id.0)
            .execute(self.pool.pg()).await?;
        Ok(r.rows_affected())
    }

    pub async fn list_outstanding(&self) -> Result<Vec<Invite>> {
        let rows: Vec<(Uuid, String, Option<String>, Option<String>,
                       OffsetDateTime, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
            r#"
            SELECT id, invited_email, display_name, created_by,
                   created_at, expires_at, redeemed_at
            FROM   invites
            WHERE  redeemed_at IS NULL AND expires_at > now()
            ORDER  BY created_at DESC
            "#).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(|r| Invite {
            id: InviteId(r.0), invited_email: r.1, display_name: r.2,
            created_by: r.3, created_at: r.4, expires_at: r.5, redeemed_at: r.6,
        }).collect())
    }

    pub async fn revoke(&self, id: InviteId) -> Result<()> {
        sqlx::query("UPDATE invites SET expires_at = now() WHERE id = $1 AND redeemed_at IS NULL")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }
}
```

- [ ] **Step 3: Register + re-export**

In `crates/teramind-db/src/repos/mod.rs`: `pub mod invite;` and `pub use invite::{Invite, InviteRepo};`.

- [ ] **Step 4: Run the test**

Run: `cargo test -p teramind-db --test invite_repo -- --nocapture`

Expected: all three PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/ids.rs \
        crates/teramind-db/src/repos/invite.rs \
        crates/teramind-db/src/repos/mod.rs \
        crates/teramind-db/tests/invite_repo.rs
git commit -m "feat(db): InviteRepo + InviteId newtype"
```

---

## Section 6 — Invite-code format

Pure-logic module. Crockford base32 over 16 random bytes (= 26 base32 chars), grouped into 4-char chunks, with a `TM-` prefix. Wire form looks like `TM-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XX` (2-char prefix + 7 separators + 26-char body = 35 chars). At rest we store `sha256(canonical_code)` only.

### Task 6.1: Failing tests

**Files:**
- Modify: `crates/teramind-sync-server/src/invite.rs` (currently a stub)

- [ ] **Step 1: Write the unit tests inside the module**

Replace the file content with:

```rust
//! Invite-code generation / parsing / hashing.

use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

const PREFIX: &str = "TM";
const RAW_BYTES: usize = 16;

#[derive(Debug, Error)]
pub enum InviteError {
    #[error("invite code must start with TM-")]
    BadPrefix,
    #[error("invite code has wrong length")]
    BadLength,
    #[error("invite code contains an invalid character")]
    BadAlphabet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCode {
    canonical: String,
}

impl InviteCode {
    pub fn generate<R: RngCore>(rng: &mut R) -> Self {
        let mut bytes = [0u8; RAW_BYTES];
        rng.fill_bytes(&mut bytes);
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: [u8; RAW_BYTES]) -> Self {
        let alphabet = base32::Alphabet::Crockford;
        let body = base32::encode(alphabet, &bytes);
        // Group into 4-char chunks for legibility.
        let mut chunks: Vec<String> = body.as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect();
        chunks.insert(0, PREFIX.into());
        Self { canonical: chunks.join("-") }
    }

    pub fn parse(input: &str) -> Result<Self, InviteError> {
        let cleaned: String = input.chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect();
        if !cleaned.starts_with(PREFIX) {
            return Err(InviteError::BadPrefix);
        }
        let body = &cleaned[PREFIX.len()..];
        // Crockford base32 of 16 bytes = ceil(16*8/5) = 26 chars.
        if body.len() != 26 { return Err(InviteError::BadLength); }
        let bytes = base32::decode(base32::Alphabet::Crockford, body)
            .ok_or(InviteError::BadAlphabet)?;
        let mut arr = [0u8; RAW_BYTES];
        if bytes.len() != RAW_BYTES { return Err(InviteError::BadLength); }
        arr.copy_from_slice(&bytes);
        Ok(Self::from_bytes(arr))
    }

    pub fn as_str(&self) -> &str { &self.canonical }

    pub fn hash(&self) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(self.canonical.as_bytes());
        h.finalize().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn generate_then_parse_roundtrips() {
        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0xC0FFEE);
        let c = InviteCode::generate(&mut rng);
        let parsed = InviteCode::parse(c.as_str()).unwrap();
        assert_eq!(c, parsed);
        assert!(c.as_str().starts_with("TM-"));
    }

    #[test]
    fn parse_is_case_insensitive_and_whitespace_tolerant() {
        let c = InviteCode::from_bytes([0x42u8; 16]);
        let lower = c.as_str().to_lowercase();
        let spaced = format!(" {lower} ");
        assert_eq!(InviteCode::parse(&spaced).unwrap(), c);
    }

    #[test]
    fn bad_prefix_errors() {
        assert!(matches!(InviteCode::parse("XX-1234-5678-9ABC-DEFG-HJKM-NPQR-STVW"),
                         Err(InviteError::BadPrefix)));
    }

    #[test]
    fn bad_length_errors() {
        assert!(matches!(InviteCode::parse("TM-1234"),
                         Err(InviteError::BadLength)));
    }

    #[test]
    fn bad_alphabet_errors() {
        // '@' is not in any base32 alphabet — guaranteed to fail decoding.
        let bad = "TM-@@@@-@@@@-@@@@-@@@@-@@@@-@@@@-@@";
        assert!(matches!(InviteCode::parse(bad), Err(InviteError::BadAlphabet)));
    }

    #[test]
    fn hash_is_stable() {
        let c = InviteCode::from_bytes([0x42u8; 16]);
        assert_eq!(c.hash(), c.hash());
        let c2 = InviteCode::from_bytes([0x43u8; 16]);
        assert_ne!(c.hash(), c2.hash());
    }
}
```

- [ ] **Step 2: Add `rand_chacha` as a dev-dep**

Edit `crates/teramind-sync-server/Cargo.toml`. Under `[dev-dependencies]`, add:

```toml
rand_chacha = { workspace = true }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p teramind-sync-server invite::`

Expected: all six pass.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-sync-server/Cargo.toml \
        crates/teramind-sync-server/src/invite.rs
git commit -m "feat(sync-server): invite code generate/parse/hash"
```

---

## Section 7 — Bearer-token format

Same pattern as invite codes, different prefix and 32 bytes of entropy. Format: `tmd_v1_<52 char crockford>`.

### Task 7.1: Implementation + tests

**Files:**
- Modify: `crates/teramind-sync-server/src/token.rs`

- [ ] **Step 1: Write the module**

```rust
//! Long-lived device bearer tokens.

use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

const PREFIX: &str = "tmd_v1_";
const RAW_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("token must start with tmd_v1_")]
    BadPrefix,
    #[error("token has wrong length")]
    BadLength,
    #[error("token contains an invalid character")]
    BadAlphabet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceToken {
    canonical: String,
}

impl DeviceToken {
    pub fn generate<R: RngCore>(rng: &mut R) -> Self {
        let mut bytes = [0u8; RAW_BYTES];
        rng.fill_bytes(&mut bytes);
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: [u8; RAW_BYTES]) -> Self {
        let body = base32::encode(base32::Alphabet::Crockford, &bytes);
        Self { canonical: format!("{PREFIX}{body}") }
    }

    pub fn parse(input: &str) -> Result<Self, TokenError> {
        let input = input.trim();
        if !input.starts_with(PREFIX) { return Err(TokenError::BadPrefix); }
        let body = &input[PREFIX.len()..];
        // base32 of 32 bytes = ceil(32*8/5) = 52 chars.
        if body.len() != 52 { return Err(TokenError::BadLength); }
        let bytes = base32::decode(base32::Alphabet::Crockford, body)
            .ok_or(TokenError::BadAlphabet)?;
        if bytes.len() != RAW_BYTES { return Err(TokenError::BadLength); }
        let mut arr = [0u8; RAW_BYTES];
        arr.copy_from_slice(&bytes);
        Ok(Self::from_bytes(arr))
    }

    pub fn as_str(&self) -> &str { &self.canonical }

    /// sha256 of the canonical wire form.
    pub fn hash(&self) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(self.canonical.as_bytes());
        h.finalize().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn roundtrips() {
        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(1234);
        let t = DeviceToken::generate(&mut rng);
        assert_eq!(DeviceToken::parse(t.as_str()).unwrap(), t);
        assert!(t.as_str().starts_with("tmd_v1_"));
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        let a = DeviceToken::from_bytes([0x10u8; 32]);
        let b = DeviceToken::from_bytes([0x11u8; 32]);
        assert_eq!(a.hash(), a.hash());
        assert_ne!(a.hash(), b.hash());
        assert_eq!(a.hash().len(), 32);
    }

    #[test]
    fn bad_prefix_errors() {
        assert!(matches!(DeviceToken::parse("xxx_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
                         Err(TokenError::BadPrefix)));
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-sync-server token::`

Expected: 3 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/src/token.rs
git commit -m "feat(sync-server): device bearer-token format"
```

---

## Section 8 — DPoP proof claims, signer, verifier

This is the security-sensitive core. Every authenticated request carries a JWS-style compact proof signed by the device's Ed25519 private key. The server verifies that the proof:
- Was signed by the **device.public_key** registered for this token.
- Names the **correct method** (`htm`) and **URL** (`htu`).
- Binds to the **token hash** (`ath`) and the **request body hash** (`bsh`).
- Has a **fresh `iat`** (within ±60 s).
- Has a **non-replayed `jti`** (replay cache; §9).

### Task 8.1: Write the failing tests

**Files:**
- Modify: `crates/teramind-sync-server/src/proof.rs`

- [ ] **Step 1: Write the proof module**

Replace the stub with:

```rust
//! DPoP-style request signing (Ed25519). RFC 9449 with our additions
//! (`ath`, `bsh`).

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofClaims {
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    pub jti: String,
    pub ath: String, // hex(sha256(bearer_token))
    pub bsh: String, // hex(sha256(request_body))
}

#[derive(Debug, Error, PartialEq)]
pub enum ProofError {
    #[error("header is not a valid 3-part JWS compact form")]
    Malformed,
    #[error("base64url decoding failed")]
    BadBase64,
    #[error("JSON claims failed to parse")]
    BadClaims,
    #[error("signature verification failed")]
    BadSignature,
    #[error("iat outside ±{0}s of now")]
    StaleIat(i64),
    #[error("htm does not match request")]
    HtmMismatch,
    #[error("htu does not match request")]
    HtuMismatch,
    #[error("ath does not match bearer token")]
    AthMismatch,
    #[error("bsh does not match body")]
    BshMismatch,
}

pub fn body_hash_hex(body: &[u8]) -> String {
    let mut h = Sha256::new(); h.update(body); hex::encode(h.finalize())
}

pub fn token_hash_hex(token: &str) -> String {
    let mut h = Sha256::new(); h.update(token.as_bytes()); hex::encode(h.finalize())
}

fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, ProofError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ProofError::BadBase64)
}

pub fn sign(claims: &ProofClaims, signing_key: &SigningKey) -> String {
    // Compact JWS: base64url(header).base64url(claims).base64url(signature)
    let header = br#"{"alg":"EdDSA","typ":"dpop+jwt"}"#;
    let claims_json = serde_json::to_vec(claims).expect("claims serialize");
    let h_b64 = b64url_encode(header);
    let c_b64 = b64url_encode(&claims_json);
    let signing_input = format!("{h_b64}.{c_b64}");
    let sig: Signature = signing_key.sign(signing_input.as_bytes());
    let s_b64 = b64url_encode(&sig.to_bytes());
    format!("{signing_input}.{s_b64}")
}

#[allow(clippy::too_many_arguments)]
pub fn verify(
    header: &str,
    public_key_bytes: &[u8],
    expected_method: &str,
    expected_url: &str,
    expected_body_hash_hex: &str,
    expected_token_hash_hex: &str,
    now_unix: i64,
    skew_secs: i64,
) -> Result<ProofClaims, ProofError> {
    let parts: Vec<&str> = header.split('.').collect();
    if parts.len() != 3 { return Err(ProofError::Malformed); }
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig_bytes = b64url_decode(parts[2])?;
    if sig_bytes.len() != 64 { return Err(ProofError::BadSignature); }
    let mut sig_arr = [0u8; 64]; sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    if public_key_bytes.len() != 32 { return Err(ProofError::BadSignature); }
    let mut pk_arr = [0u8; 32]; pk_arr.copy_from_slice(public_key_bytes);
    let pk = VerifyingKey::from_bytes(&pk_arr).map_err(|_| ProofError::BadSignature)?;
    pk.verify(signing_input.as_bytes(), &sig).map_err(|_| ProofError::BadSignature)?;

    let claims_bytes = b64url_decode(parts[1])?;
    let claims: ProofClaims = serde_json::from_slice(&claims_bytes).map_err(|_| ProofError::BadClaims)?;

    if claims.htm != expected_method { return Err(ProofError::HtmMismatch); }
    if claims.htu != expected_url    { return Err(ProofError::HtuMismatch); }
    if claims.ath != expected_token_hash_hex { return Err(ProofError::AthMismatch); }
    if claims.bsh != expected_body_hash_hex  { return Err(ProofError::BshMismatch); }
    if (now_unix - claims.iat).abs() > skew_secs { return Err(ProofError::StaleIat(skew_secs)); }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::{rngs::OsRng, RngCore};

    fn fresh_keypair() -> (SigningKey, Vec<u8>) {
        let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes().to_vec();
        (sk, pk)
    }

    fn happy_claims(token: &str, body: &[u8], now: i64) -> ProofClaims {
        ProofClaims {
            htm: "POST".into(),
            htu: "https://srv/v1/ingest".into(),
            iat: now,
            jti: "deadbeef0123".into(),
            ath: token_hash_hex(token),
            bsh: body_hash_hex(body),
        }
    }

    #[test]
    fn sign_then_verify_happy() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let token = "tmd_v1_AAAA";
        let body = br#"{"x":1}"#;
        let c = happy_claims(token, body, now);
        let header = sign(&c, &sk);
        let out = verify(&header, &pk, "POST", "https://srv/v1/ingest",
                         &body_hash_hex(body), &token_hash_hex(token), now, 60).unwrap();
        assert_eq!(out.jti, "deadbeef0123");
    }

    #[test]
    fn wrong_public_key_fails() {
        let (sk, _) = fresh_keypair();
        let (_, other_pk) = fresh_keypair();
        let now = 1_700_000_000;
        let body = b"";
        let c = happy_claims("tmd_v1_X", body, now);
        let header = sign(&c, &sk);
        let err = verify(&header, &other_pk, "POST", "https://srv/v1/ingest",
                         &body_hash_hex(body), &token_hash_hex("tmd_v1_X"), now, 60).unwrap_err();
        assert_eq!(err, ProofError::BadSignature);
    }

    #[test]
    fn wrong_method_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let header = sign(&c, &sk);
        let err = verify(&header, &pk, "GET", "https://srv/v1/ingest",
                         &body_hash_hex(b""), &token_hash_hex("tmd_v1_X"), now, 60).unwrap_err();
        assert_eq!(err, ProofError::HtmMismatch);
    }

    #[test]
    fn wrong_url_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let header = sign(&c, &sk);
        let err = verify(&header, &pk, "POST", "https://srv/v1/rpc",
                         &body_hash_hex(b""), &token_hash_hex("tmd_v1_X"), now, 60).unwrap_err();
        assert_eq!(err, ProofError::HtuMismatch);
    }

    #[test]
    fn tampered_body_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"clean", now);
        let header = sign(&c, &sk);
        let err = verify(&header, &pk, "POST", "https://srv/v1/ingest",
                         &body_hash_hex(b"tampered"), &token_hash_hex("tmd_v1_X"), now, 60).unwrap_err();
        assert_eq!(err, ProofError::BshMismatch);
    }

    #[test]
    fn token_mismatch_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let header = sign(&c, &sk);
        let err = verify(&header, &pk, "POST", "https://srv/v1/ingest",
                         &body_hash_hex(b""), &token_hash_hex("tmd_v1_OTHER"), now, 60).unwrap_err();
        assert_eq!(err, ProofError::AthMismatch);
    }

    #[test]
    fn stale_iat_fails() {
        let (sk, pk) = fresh_keypair();
        let signed_at = 1_700_000_000;
        let way_later = signed_at + 120;
        let c = happy_claims("tmd_v1_X", b"", signed_at);
        let header = sign(&c, &sk);
        let err = verify(&header, &pk, "POST", "https://srv/v1/ingest",
                         &body_hash_hex(b""), &token_hash_hex("tmd_v1_X"), way_later, 60).unwrap_err();
        assert_eq!(err, ProofError::StaleIat(60));
    }

    #[test]
    fn flipped_signature_byte_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let mut header = sign(&c, &sk);
        // Flip the last char of the signature segment.
        let last = header.pop().unwrap();
        let new = if last == 'A' { 'B' } else { 'A' };
        header.push(new);
        let err = verify(&header, &pk, "POST", "https://srv/v1/ingest",
                         &body_hash_hex(b""), &token_hash_hex("tmd_v1_X"), now, 60).unwrap_err();
        assert_eq!(err, ProofError::BadSignature);
    }
}
```

- [ ] **Step 2: Add `base64` to the crate deps**

Edit `crates/teramind-sync-server/Cargo.toml`. Under `[dependencies]` add:

```toml
base64 = "0.22"
```

(And to workspace deps in root Cargo.toml if not already present:)

```toml
base64 = "0.22"
```

Then change the crate's dep line to `base64 = { workspace = true }`.

- [ ] **Step 3: Run the tests**

Run: `cargo test -p teramind-sync-server proof::`

Expected: 8 PASS.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml \
        crates/teramind-sync-server/Cargo.toml \
        crates/teramind-sync-server/src/proof.rs
git commit -m "feat(sync-server): DPoP proof claims + Ed25519 sign/verify"
```

---

## Section 9 — DPoP replay cache

A per-device LRU of seen `jti` values, bounded at 10 000 entries with entries expiring at 60 s.

### Task 9.1: Failing tests

**Files:**
- Modify: `crates/teramind-sync-server/src/proof.rs` (append a `replay` submodule)

- [ ] **Step 1: Append the replay-cache module + tests**

At the bottom of `proof.rs`, add:

```rust
pub mod replay {
    use parking_lot::Mutex;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use teramind_core::ids::DeviceId;

    pub struct ReplayCache {
        max_per_device: usize,
        ttl: Duration,
        // (jti, inserted_at)
        inner: Mutex<HashMap<DeviceId, VecDeque<(String, Instant)>>>,
    }

    impl ReplayCache {
        pub fn new(max_per_device: usize, ttl_secs: u64) -> Arc<Self> {
            Arc::new(Self {
                max_per_device,
                ttl: Duration::from_secs(ttl_secs),
                inner: Mutex::new(HashMap::new()),
            })
        }

        /// Returns true if `jti` was newly inserted; false if it's a replay.
        pub fn check_and_insert(&self, device: DeviceId, jti: &str) -> bool {
            let now = Instant::now();
            let mut map = self.inner.lock();
            let q = map.entry(device).or_default();

            // Drop expired entries from the front.
            while let Some((_, ts)) = q.front() {
                if now.duration_since(*ts) > self.ttl { q.pop_front(); } else { break; }
            }

            if q.iter().any(|(j, _)| j == jti) { return false; }

            q.push_back((jti.to_string(), now));
            while q.len() > self.max_per_device { q.pop_front(); }
            true
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use teramind_core::ids::DeviceId;
        use uuid::Uuid;

        #[test]
        fn first_insert_returns_true_replay_returns_false() {
            let c = ReplayCache::new(8, 60);
            let d = DeviceId(Uuid::new_v4());
            assert!(c.check_and_insert(d, "j1"));
            assert!(!c.check_and_insert(d, "j1"));
            assert!(c.check_and_insert(d, "j2"));
        }

        #[test]
        fn distinct_devices_are_isolated() {
            let c = ReplayCache::new(8, 60);
            let a = DeviceId(Uuid::new_v4());
            let b = DeviceId(Uuid::new_v4());
            assert!(c.check_and_insert(a, "j1"));
            assert!(c.check_and_insert(b, "j1"));
        }

        #[test]
        fn cap_evicts_oldest() {
            let c = ReplayCache::new(2, 60);
            let d = DeviceId(Uuid::new_v4());
            assert!(c.check_and_insert(d, "j1"));
            assert!(c.check_and_insert(d, "j2"));
            assert!(c.check_and_insert(d, "j3"));
            // j1 should have been evicted by capacity; a re-insert succeeds.
            assert!(c.check_and_insert(d, "j1"));
        }
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p teramind-sync-server proof::replay::`

Expected: 3 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/src/proof.rs
git commit -m "feat(sync-server): per-device DPoP replay cache"
```

---

## Section 10 — ServerConfig (TOML loader)

### Task 10.1: Implement + test

**Files:**
- Modify: `crates/teramind-sync-server/src/config.rs`

- [ ] **Step 1: Write the config module**

Replace the stub with:

```rust
//! Server configuration loaded from TOML.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub database_url: String,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub ingest: IngestConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "AuthConfig::default_invite_expiry_days")]
    pub invite_default_expires_days: i64,
    #[serde(default = "AuthConfig::default_replay_window")]
    pub proof_replay_window_secs: i64,
    #[serde(default = "AuthConfig::default_replay_size")]
    pub proof_replay_cache_size: usize,
}

impl AuthConfig {
    fn default_invite_expiry_days() -> i64 { 7 }
    fn default_replay_window() -> i64 { 60 }
    fn default_replay_size() -> usize { 10_000 }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            invite_default_expires_days: Self::default_invite_expiry_days(),
            proof_replay_window_secs: Self::default_replay_window(),
            proof_replay_cache_size: Self::default_replay_size(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IngestConfig {
    #[serde(default = "IngestConfig::default_batch")]
    pub max_batch_size: usize,
    #[serde(default = "IngestConfig::default_body")]
    pub max_request_body_bytes: usize,
}

impl IngestConfig {
    fn default_batch() -> usize { 32 }
    fn default_body() -> usize { 10 * 1024 * 1024 }
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self { max_batch_size: Self::default_batch(), max_request_body_bytes: Self::default_body() }
    }
}

impl ServerConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        let cfg: ServerConfig = toml::from_str(&s)?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_minimal() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"
listen_addr  = "0.0.0.0:443"
database_url = "postgres://u:p@h/db"
"#).unwrap();
        let cfg = ServerConfig::load(f.path()).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:443");
        assert!(cfg.tls.is_none());
        assert_eq!(cfg.auth.invite_default_expires_days, 7);
        assert_eq!(cfg.ingest.max_batch_size, 32);
    }

    #[test]
    fn loads_with_tls_and_overrides() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"
listen_addr  = "0.0.0.0:443"
database_url = "postgres://u:p@h/db"

[tls]
cert_file = "/etc/cert.pem"
key_file  = "/etc/key.pem"

[auth]
proof_replay_window_secs = 30
"#).unwrap();
        let cfg = ServerConfig::load(f.path()).unwrap();
        assert!(cfg.tls.is_some());
        assert_eq!(cfg.auth.proof_replay_window_secs, 30);
        assert_eq!(cfg.auth.proof_replay_cache_size, 10_000);
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-sync-server config::`

Expected: 2 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/src/config.rs
git commit -m "feat(sync-server): TOML ServerConfig"
```

---

## Section 11 — AppState

The shared state injected into every handler.

### Task 11.1: Implementation

**Files:**
- Modify: `crates/teramind-sync-server/src/state.rs`

- [ ] **Step 1: Write the module**

Replace the stub with:

```rust
//! Shared application state passed to every handler.

use crate::config::ServerConfig;
use crate::proof::replay::ReplayCache;
use std::sync::Arc;
use teramind_db::pool::DbPool;
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use teramind_core::ids::{DeviceId, UserId};

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub users: UserRepo,
    pub devices: DeviceRepo,
    pub invites: InviteRepo,
    pub replay: Arc<ReplayCache>,
    pub cfg: Arc<ServerConfig>,
}

#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: UserId,
    pub device_id: DeviceId,
}

impl AppState {
    pub fn new(pool: DbPool, cfg: ServerConfig) -> Self {
        let replay = ReplayCache::new(
            cfg.auth.proof_replay_cache_size,
            cfg.auth.proof_replay_window_secs as u64,
        );
        Self {
            users: UserRepo::new(pool.clone()),
            devices: DeviceRepo::new(pool.clone()),
            invites: InviteRepo::new(pool.clone()),
            pool,
            replay,
            cfg: Arc::new(cfg),
        }
    }
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p teramind-sync-server`

Expected: success.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/src/state.rs
git commit -m "feat(sync-server): AppState + AuthContext"
```

---

## Section 12 — Health + version endpoints + server scaffold

### Task 12.1: Health handler

**Files:**
- Modify: `crates/teramind-sync-server/src/handlers/mod.rs`
- Modify: `crates/teramind-sync-server/src/handlers/health.rs` (currently a stub — create at this path if not yet present)

- [ ] **Step 1: Write health handler**

Create `crates/teramind-sync-server/src/handlers/health.rs`:

```rust
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use crate::state::AppState;

#[derive(Serialize)]
struct Health { status: &'static str, db: &'static str }

#[derive(Serialize)]
struct Version { version: &'static str }

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.pool.pg()).await.is_ok();
    let body = Health {
        status: if ok { "ok" } else { "degraded" },
        db:     if ok { "ok" } else { "down" },
    };
    if ok { (StatusCode::OK, Json(body)) } else { (StatusCode::SERVICE_UNAVAILABLE, Json(body)) }
}

pub async fn version() -> impl IntoResponse {
    Json(Version { version: crate::VERSION })
}
```

- [ ] **Step 2: Register the module**

Edit `crates/teramind-sync-server/src/handlers/mod.rs`:

```rust
//! HTTP handlers.
pub mod health;
pub mod redeem;
pub mod ingest;
```

- [ ] **Step 3: Stub the other handler modules so the module tree compiles**

Create `crates/teramind-sync-server/src/handlers/redeem.rs`:
```rust
//! Placeholder; populated in §14.
```

Create `crates/teramind-sync-server/src/handlers/ingest.rs`:
```rust
//! Placeholder; populated in §16.
```

(Note: `lib.rs` declared `pub mod handlers;` in §1.3 — the directory and `mod.rs` were created there. This task just fills in the per-handler files.)

### Task 12.2: Server scaffold

**Files:**
- Modify: `crates/teramind-sync-server/src/server.rs`

- [ ] **Step 1: Write the router**

Replace the stub with:

```rust
//! axum app construction + listener.

use crate::handlers;
use crate::state::AppState;
use axum::{routing::get, Router};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::info;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health",  get(handlers::health::health))
        .route("/v1/version", get(handlers::health::version))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(state: AppState, addr: SocketAddr) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "teramind-sync-server listening (HTTP)");
    axum::serve(listener, app).await?;
    Ok(())
}
```

### Task 12.3: Wire `serve` into the CLI

**Files:**
- Modify: `crates/teramind-sync-server/src/main.rs`

- [ ] **Step 1: Replace main with the multi-subcommand entry**

```rust
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "teramind-sync-server", version)]
struct Cli {
    /// Path to config TOML (defaults to /etc/teramind-sync-server/config.toml).
    #[arg(long, env = "TERAMIND_SYNC_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run database migrations against the configured Postgres.
    Migrate,
    /// Start the HTTP(S) server.
    Serve {
        /// Bind only HTTP (no TLS). Insecure; loud flag for dev only.
        #[arg(long)]
        insecure_allow_http: bool,
    },
    /// Print version.
    Version,
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TERAMIND_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json().init();
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logging();
    match cli.cmd {
        Cmd::Version => {
            println!("teramind-sync-server {}", teramind_sync_server::VERSION);
            Ok(())
        }
        Cmd::Migrate => {
            let cfg_path = cli.config.unwrap_or_else(default_config_path);
            let cfg = teramind_sync_server::config::ServerConfig::load(&cfg_path)?;
            let pool = teramind_db::pool::DbPool::connect_url(&cfg.database_url).await?;
            teramind_db::migrate::run(&pool).await?;
            println!("migrations OK");
            Ok(())
        }
        Cmd::Serve { insecure_allow_http } => {
            let cfg_path = cli.config.unwrap_or_else(default_config_path);
            let cfg = teramind_sync_server::config::ServerConfig::load(&cfg_path)?;
            if cfg.tls.is_none() && !insecure_allow_http {
                anyhow::bail!("TLS not configured; pass --insecure-allow-http to opt into plaintext HTTP (dev only)");
            }
            let pool = teramind_db::pool::DbPool::connect_url(&cfg.database_url).await?;
            teramind_db::migrate::run(&pool).await?;
            let addr: SocketAddr = cfg.listen_addr.parse()?;
            let state = teramind_sync_server::state::AppState::new(pool, cfg.clone());
            if let Some(tls) = cfg.tls.as_ref() {
                teramind_sync_server::server::serve_tls(state, addr, tls).await
            } else {
                teramind_sync_server::server::serve(state, addr).await
            }
        }
    }
}

fn default_config_path() -> PathBuf {
    PathBuf::from("/etc/teramind-sync-server/config.toml")
}
```

- [ ] **Step 2: Add `DbPool::connect_url`**

Inspect `crates/teramind-db/src/pool.rs`. If a `connect_url(url: &str)` constructor does not already exist, add it. (It accepts a `postgres://…` URL and returns `DbPool`.) Existing code uses `connect(connect_options)`; add an adapter:

```rust
// Append to crates/teramind-db/src/pool.rs
impl DbPool {
    pub async fn connect_url(url: &str) -> anyhow::Result<Self> {
        let opts: sqlx::postgres::PgConnectOptions = url.parse()?;
        Self::connect(opts).await.map_err(Into::into)
    }
}
```

- [ ] **Step 3: TLS stub**

Append a TLS serve stub to `crates/teramind-sync-server/src/server.rs` so this compiles. The real TLS wiring is §20:

```rust
pub async fn serve_tls(
    state: AppState,
    addr: SocketAddr,
    _tls: &crate::config::TlsConfig,
) -> anyhow::Result<()> {
    // Replaced with real rustls/axum-server wiring in §20.
    serve(state, addr).await
}
```

- [ ] **Step 4: Build**

Run: `cargo build -p teramind-sync-server`

Expected: success.

### Task 12.4: Smoke-test health + version via the running server

**Files:**
- Create: `crates/teramind-sync-server/tests/health.rs`

- [ ] **Step 1: Write a black-box health test**

```rust
//! Spins up the server against an embedded PG and hits /v1/health + /v1/version.

use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_returns_ok_when_db_up() -> anyhow::Result<()> {
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
    };
    let state = AppState::new(pool, cfg);
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let resp = reqwest::get(format!("http://{addr}/v1/health")).await?;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["status"], "ok");

    let ver = reqwest::get(format!("http://{addr}/v1/version")).await?
        .json::<serde_json::Value>().await?;
    assert_eq!(ver["version"], env!("CARGO_PKG_VERSION"));

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-sync-server --test health -- --nocapture`

Expected: PASS (~15s with PG warmup).

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-db/src/pool.rs \
        crates/teramind-sync-server/src/handlers \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/src/main.rs \
        crates/teramind-sync-server/tests/health.rs
git commit -m "feat(sync-server): axum scaffold + health + version + CLI subcommands"
```

---

## Section 13 — Auth middleware (Tower layer)

The middleware sits in front of every authenticated route. It:
1. Parses `Authorization: Bearer ...` → `sha256(token)` → `DeviceRepo::get_active_by_token_hash`.
2. Parses `X-Teramind-Proof: ...` → DPoP-verifies against `device.public_key`, the matched method/URL, the token hash, the body hash, and current time.
3. Calls `replay::check_and_insert(device_id, jti)`; rejects on replay.
4. Attaches `AuthContext { user_id, device_id }` as an axum request extension.
5. Spawns a fire-and-forget task to `touch_last_seen(device_id)`.

### Task 13.1: Tests

**Files:**
- Create: `crates/teramind-sync-server/tests/auth_middleware.rs`

- [ ] **Step 1: Write tests**

```rust
//! Black-box tests for the auth middleware. Mounts an echo handler behind the
//! middleware on a random port and asserts 401 / 403 / 200 cases.

use axum::{routing::post, Extension, Json, Router};
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::{DeviceRepo, UserRepo}};
use teramind_sync_server::auth::auth_middleware;
use teramind_sync_server::config::*;
use teramind_sync_server::proof::{sign, ProofClaims, body_hash_hex, token_hash_hex};
use teramind_sync_server::state::{AppState, AuthContext};
use teramind_sync_server::token::DeviceToken;
use time::OffsetDateTime;

async fn echo(Extension(auth): Extension<AuthContext>) -> Json<serde_json::Value> {
    Json(json!({ "user": auth.user_id.0.to_string(), "device": auth.device_id.0.to_string() }))
}

fn fresh_signing_key() -> (SigningKey, Vec<u8>) {
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    (sk, pk)
}

async fn boot() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, AppState)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(), database_url: "ignored".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool, cfg);
    let app = Router::new()
        .route("/v1/echo", post(echo))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    Ok((dir, sup, addr, state))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_authorization_is_401() -> anyhow::Result<()> {
    let (_d, sup, addr, _s) = boot().await?;
    let r = reqwest::Client::new().post(format!("http://{addr}/v1/echo"))
        .body("{}").send().await?;
    assert_eq!(r.status(), 401);
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn valid_bearer_plus_proof_passes() -> anyhow::Result<()> {
    let (_d, sup, addr, state) = boot().await?;

    // Register a user + device.
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (sk, pk) = fresh_signing_key();
    devices.insert(user.id, "alice-mac", &token.hash(), &pk).await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    let body = br#"{"hello":1}"#;
    let url = format!("http://{addr}/v1/echo");
    let claims = ProofClaims {
        htm: "POST".into(), htu: url.clone(), iat: now,
        jti: "test-jti-1".into(),
        ath: token_hash_hex(token.as_str()),
        bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &sk);

    let r = reqwest::Client::new().post(&url)
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .header("X-Teramind-Proof", proof)
        .header("Content-Type", "application/json")
        .body(body.to_vec()).send().await?;
    assert_eq!(r.status(), 200, "happy path must pass");
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bearer_without_proof_is_403() -> anyhow::Result<()> {
    let (_d, sup, addr, state) = boot().await?;
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (_sk, pk) = fresh_signing_key();
    devices.insert(user.id, "alice-mac", &token.hash(), &pk).await?;

    let r = reqwest::Client::new().post(format!("http://{addr}/v1/echo"))
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .body("{}").send().await?;
    assert_eq!(r.status(), 403);
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proof_with_wrong_key_is_403() -> anyhow::Result<()> {
    let (_d, sup, addr, state) = boot().await?;
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (_registered_sk, registered_pk) = fresh_signing_key();
    let (attacker_sk, _) = fresh_signing_key();
    devices.insert(user.id, "alice-mac", &token.hash(), &registered_pk).await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    let body = br#"{}"#;
    let url = format!("http://{addr}/v1/echo");
    let claims = ProofClaims {
        htm: "POST".into(), htu: url.clone(), iat: now,
        jti: "test-attack-jti".into(),
        ath: token_hash_hex(token.as_str()), bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &attacker_sk);

    let r = reqwest::Client::new().post(&url)
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .header("X-Teramind-Proof", proof).body(body.to_vec()).send().await?;
    assert_eq!(r.status(), 403, "stolen token without matching key must fail");
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replayed_jti_is_403() -> anyhow::Result<()> {
    let (_d, sup, addr, state) = boot().await?;
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (sk, pk) = fresh_signing_key();
    devices.insert(user.id, "alice-mac", &token.hash(), &pk).await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    let body = br#"{}"#;
    let url = format!("http://{addr}/v1/echo");
    let claims = ProofClaims {
        htm: "POST".into(), htu: url.clone(), iat: now,
        jti: "fixed-jti".into(),
        ath: token_hash_hex(token.as_str()), bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &sk);

    let client = reqwest::Client::new();
    let h = |p: &str| client.post(&url)
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .header("X-Teramind-Proof", p).body(body.to_vec());
    let first = h(&proof).send().await?;
    assert_eq!(first.status(), 200);
    let second = h(&proof).send().await?;
    assert_eq!(second.status(), 403, "replayed jti must fail");
    sup.shutdown().await?; Ok(())
}
```

- [ ] **Step 2: Run, watch them fail**

Run: `cargo test -p teramind-sync-server --test auth_middleware`

Expected: FAIL — `auth_middleware` symbol not defined.

### Task 13.2: Implement the middleware

**Files:**
- Modify: `crates/teramind-sync-server/src/auth.rs`

- [ ] **Step 1: Write the middleware**

Replace the stub with:

```rust
//! Tower middleware: parse bearer + DPoP proof, attach AuthContext.

use crate::proof::{body_hash_hex, token_hash_hex, verify};
use crate::state::{AppState, AuthContext};
use crate::token::DeviceToken;
use axum::{
    body::Body, extract::{Request, State},
    http::{header, StatusCode}, middleware::Next, response::Response,
};
use sha2::{Digest, Sha256};
use std::time::Duration;
use time::OffsetDateTime;

pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let (mut parts, body) = request.into_parts();

    // 1. Parse Authorization.
    let bearer = parts.headers.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_string();
    let token = DeviceToken::parse(&bearer).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let token_hash = token.hash();
    let device = state.devices.get_active_by_token_hash(&token_hash).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // 2. Read X-Teramind-Proof + buffer body for hashing.
    let proof = parts.headers.get("x-teramind-proof")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?.to_string();
    let body_bytes = axum::body::to_bytes(body, state.cfg.ingest.max_request_body_bytes).await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let body_hash = body_hash_hex(&body_bytes);

    // 3. Verify proof.
    let url = matched_url(&parts);
    let method = parts.method.as_str().to_string();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = verify(
        &proof, &device.public_key, &method, &url,
        &body_hash, &token_hash_hex(token.as_str()),
        now, state.cfg.auth.proof_replay_window_secs,
    ).map_err(|_| StatusCode::FORBIDDEN)?;

    // 4. Replay check.
    if !state.replay.check_and_insert(device.id, &claims.jti) {
        return Err(StatusCode::FORBIDDEN);
    }

    // 5. Fire-and-forget last-seen update.
    {
        let devices = state.devices.clone();
        let did = device.id;
        tokio::spawn(async move { let _ = devices.touch_last_seen(did).await; });
    }

    // 6. Attach AuthContext and rebuild the request.
    parts.extensions.insert(AuthContext {
        user_id: device.user_id, device_id: device.id,
    });
    let req = Request::from_parts(parts, Body::from(body_bytes));
    Ok(next.run(req).await)
}

/// Build the canonical absolute URL the client signed against.
fn matched_url(parts: &http::request::Parts) -> String {
    // `htu` must match exactly what the client signed.
    // The client signs the full URL `https://host/path` (no query string by convention).
    // We reconstruct from Host header + scheme heuristic.
    let scheme = if parts.headers.get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok()) == Some("https")
        || parts.uri.scheme_str() == Some("https") { "https" } else { "http" };
    let host = parts.headers.get(header::HOST)
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    let path = parts.uri.path();
    format!("{scheme}://{host}{path}")
}
```

- [ ] **Step 2: Add `http` crate to crate deps**

axum re-exports `http`; ensure it's in scope. Add to `Cargo.toml`:

```toml
http = "1"
```

(plus workspace dep if needed).

- [ ] **Step 3: Run**

Run: `cargo test -p teramind-sync-server --test auth_middleware -- --nocapture`

Expected: all 5 PASS.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml \
        crates/teramind-sync-server/Cargo.toml \
        crates/teramind-sync-server/src/auth.rs \
        crates/teramind-sync-server/tests/auth_middleware.rs
git commit -m "feat(sync-server): tower auth middleware (bearer + DPoP + replay)"
```

---

## Section 14 — POST /v1/auth/redeem

### Task 14.1: Failing test

**Files:**
- Create: `crates/teramind-sync-server/tests/redeem.rs`

- [ ] **Step 1: Test the happy path + 3 failure modes**

```rust
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::repos::InviteRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::invite::InviteCode;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::{Duration as TDur, OffsetDateTime};

async fn boot() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(), database_url: "ignored".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    Ok((dir, sup, addr, pool))
}

fn fresh_pk() -> Vec<u8> {
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    SigningKey::from_bytes(&seed).verifying_key().to_bytes().to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy_path_issues_token() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let invites = InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x11u8; 16]);
    invites.create(&code.hash(), "alice@acme.dev", Some("Alice"), None,
                   OffsetDateTime::now_utc() + TDur::days(7)).await?;
    let pk = fresh_pk();

    let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(),
            "device_name": "alice-mac",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        })).send().await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert!(body["device_token"].as_str().unwrap().starts_with("tmd_v1_"));
    assert!(body["user_id"].is_string());
    assert!(body["device_id"].is_string());

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redeeming_twice_is_409() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let invites = InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x12u8; 16]);
    invites.create(&code.hash(), "alice@acme.dev", None, None,
                   OffsetDateTime::now_utc() + TDur::days(7)).await?;
    let pk = fresh_pk();

    let send = || async {
        reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
            .json(&json!({
                "invite_code": code.as_str(),
                "device_name": "x",
                "device_public_key_b64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD, &pk),
            })).send().await.unwrap()
    };
    assert_eq!(send().await.status(), 200);
    assert_eq!(send().await.status(), 409);

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_invite_is_410() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let invites = InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x13u8; 16]);
    invites.create(&code.hash(), "x@acme.dev", None, None,
                   OffsetDateTime::now_utc() - TDur::seconds(1)).await?;
    let pk = fresh_pk();
    let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "x",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        })).send().await?;
    assert_eq!(r.status(), 410);
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_code_is_400() -> anyhow::Result<()> {
    let (_d, sup, addr, _pool) = boot().await?;
    let pk = fresh_pk();
    let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": "garbage", "device_name": "x",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        })).send().await?;
    assert_eq!(r.status(), 400);
    sup.shutdown().await?; Ok(())
}
```

- [ ] **Step 2: Run, watch fail**

Run: `cargo test -p teramind-sync-server --test redeem`

Expected: FAIL (route returns 404 — not yet registered).

### Task 14.2: Implement /v1/auth/redeem

**Files:**
- Modify: `crates/teramind-sync-server/src/handlers/redeem.rs`
- Modify: `crates/teramind-sync-server/src/server.rs` (add route)

- [ ] **Step 1: Write the handler**

```rust
//! POST /v1/auth/redeem — exchange an invite code + a device public key for
//! a long-lived bearer token. Atomic transaction: upsert user, insert device,
//! mark invite redeemed.

use crate::invite::InviteCode;
use crate::state::AppState;
use crate::token::DeviceToken;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use teramind_db::repos::Device;

#[derive(Deserialize)]
pub struct RedeemRequest {
    pub invite_code: String,
    pub device_name: String,
    pub device_public_key_b64: String,
}

#[derive(Serialize)]
pub struct RedeemResponse {
    pub user_id: String,
    pub device_id: String,
    pub device_token: String,
    pub device_name: String,
}

pub async fn redeem(
    State(state): State<AppState>,
    Json(req): Json<RedeemRequest>,
) -> Result<(StatusCode, Json<RedeemResponse>), (StatusCode, String)> {
    // Parse invite + device key.
    let code = InviteCode::parse(&req.invite_code)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad invite: {e}")))?;
    let pk = B64.decode(&req.device_public_key_b64)
        .map_err(|_| (StatusCode::BAD_REQUEST, "device_public_key_b64 must be base64".into()))?;
    if pk.len() != 32 {
        return Err((StatusCode::BAD_REQUEST, "device_public_key must be 32 bytes".into()));
    }
    if req.device_name.is_empty() || req.device_name.len() > 200 {
        return Err((StatusCode::BAD_REQUEST, "device_name length 1..=200".into()));
    }

    // Look up the invite.
    let invite = state.invites.find_redeemable(&code.hash()).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?
        .ok_or_else(|| {
            // Disambiguate expired vs missing for a better operator signal.
            // (Cheap secondary lookup — acceptable cost in the unhappy path.)
            (StatusCode::GONE, "invite not redeemable (missing or expired)".into())
        })?;

    // Upsert user.
    let user = state.users.upsert_by_email(&invite.invited_email, invite.display_name.as_deref())
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;

    // Generate token + insert device.
    let token = DeviceToken::generate(&mut OsRng);
    let device: Device = state.devices.insert(user.id, &req.device_name, &token.hash(), &pk)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;

    // Mark invite redeemed; rows_affected == 0 ⇒ race ⇒ 409.
    let n = state.invites.mark_redeemed(&code.hash(), device.id).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;
    if n == 0 {
        // Roll back the device we just created so the user can't accumulate
        // ghosts on a race.
        let _ = state.devices.revoke(device.id).await;
        return Err((StatusCode::CONFLICT, "invite already redeemed".into()));
    }

    Ok((StatusCode::OK, Json(RedeemResponse {
        user_id: user.id.0.to_string(),
        device_id: device.id.0.to_string(),
        device_token: token.as_str().to_string(),
        device_name: device.name,
    })))
}
```

Note: the test for "expired" uses an invite created with `expires_at` in the past; `find_redeemable` returns `None` for it; we map `None` to 410. Tests for missing-code also hit 410 — that's accepted; the unhappy disambiguation between "missing" and "expired" is operator-friendly but the test contract is "non-redeemable → 410". Cross-check that the `malformed_code_is_400` test passes because parsing fails first.

- [ ] **Step 2: Wire the route**

In `crates/teramind-sync-server/src/server.rs`, replace the `build_router` body to also register the redeem route:

```rust
pub fn build_router(state: AppState) -> Router {
    use axum::routing::{get, post};
    Router::new()
        .route("/v1/health",        get(handlers::health::health))
        .route("/v1/version",       get(handlers::health::version))
        .route("/v1/auth/redeem",   post(handlers::redeem::redeem))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramind-sync-server --test redeem -- --nocapture`

Expected: 4 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-sync-server/src/handlers/redeem.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/redeem.rs
git commit -m "feat(sync-server): POST /v1/auth/redeem"
```

---

## Section 15 — Extract `route()` for reuse

The daemon's `services::ingest::route()` is a private fn that dispatches an `EventEnvelope` to per-event-type handlers. The server needs to call exactly this dispatch logic, but with `(user_id, device_id)` annotation. Promote it to a public reusable fn parameterised by a `RouteDeps` struct.

### Task 15.1: Make the existing call still pass

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`
- Modify: `crates/teramindd/src/lib.rs`

- [ ] **Step 1: Read the current `route` signature**

Run: `grep -n 'async fn route' crates/teramindd/src/services/ingest.rs`

Expected: a single private fn `async fn route(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()>`.

- [ ] **Step 2: Introduce `RouteDeps` and a public `route_with_deps`**

Edit `crates/teramindd/src/services/ingest.rs`. After the `IngestDeps` definition (around line 32), add:

```rust
/// Subset of IngestDeps that the dispatch fn actually needs. Used by both
/// the daemon (which wraps it in IngestDeps) and the sync server (which
/// constructs it directly).
#[derive(Clone)]
pub struct RouteDeps {
    pub sessions: crate::services::session_manager::SessionManager,
    pub agents: teramind_db::repos::AgentRepo,
    pub session_repo: teramind_db::repos::SessionRepo,
    pub trace: teramind_db::repos::TraceRepo,
    pub diffs: teramind_db::repos::DiffRepo,
    pub fs_registry: std::sync::Arc<crate::services::fs_watcher::WatchRegistry>,
    pub write_tool_ring: crate::services::write_tool_ring::WriteToolRing,
}

impl From<&IngestDeps> for RouteDeps {
    fn from(d: &IngestDeps) -> Self {
        Self {
            sessions: d.sessions.clone(),
            agents: d.agents.clone(),
            session_repo: d.session_repo.clone(),
            trace: d.trace.clone(),
            diffs: d.diffs.clone(),
            fs_registry: d.fs_registry.clone(),
            write_tool_ring: d.write_tool_ring.clone(),
        }
    }
}

/// `(user_id, device_id)` annotation for server-side ingest. The daemon
/// passes `None`; the server passes `Some(...)`.
#[derive(Debug, Clone, Copy)]
pub struct IngestAuth {
    pub user_id: uuid::Uuid,
    pub device_id: uuid::Uuid,
}

/// Public dispatch entry point. Same body as the old `route()` but uses
/// `RouteDeps` + `IngestAuth`. The daemon path passes `auth = None`.
pub async fn route_with_deps(
    d: &RouteDeps,
    env: teramind_core::types::ingest_event::EventEnvelope,
    auth: Option<IngestAuth>,
) -> anyhow::Result<()> {
    route_inner(d, env, auth).await
}
```

- [ ] **Step 3: Replace the existing private `route` with a wrapper**

Find the existing `async fn route(d: &IngestDeps, env: EventEnvelope)` definition. Rename it to `route_inner` and change the first arg type from `&IngestDeps` to `&RouteDeps`, then thread an `auth: Option<IngestAuth>` argument through. Inside the body, wherever a `SessionStart` or `Skill`-related INSERT happens, pipe `auth.map(|a| a.user_id)` and `auth.map(|a| a.device_id)` into the SQL.

Concretely, the only IngestEvent that produces a fresh `sessions` row is `SessionStart`. Locate the `sessions.insert(NewSession { … })` call inside the `SessionStart` arm. **You will need to:**

1. Extend `crates/teramind-db/src/repos/session.rs::NewSession<'a>` with two new optional fields:

```rust
pub struct NewSession<'a> {
    pub agent_id: AgentId,
    pub agent_session_id: Option<&'a str>,
    pub cwd: &'a str,
    pub project_id: Option<ProjectId>,
    pub parent_session_id: Option<SessionId>,
    pub git_head: Option<&'a str>,
    pub git_branch: Option<&'a str>,
    pub os: &'a str,
    pub hostname: &'a str,
    pub user_login: &'a str,
    pub started_at: OffsetDateTime,
    pub user_id:   Option<UserId>,
    pub device_id: Option<DeviceId>,
}
```

2. Extend `SessionRepo::insert`'s SQL + bindings to include the two columns:

```rust
INSERT INTO sessions (agent_id, agent_session_id, cwd, project_id, parent_session_id,
                      git_head, git_branch, os, hostname, user_login, started_at,
                      user_id, device_id)
VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
```

…and `.bind(n.user_id.map(|u| u.0)).bind(n.device_id.map(|d| d.0))`.

3. In the `SessionStart` arm of `route_inner`, populate the new fields:

```rust
let new = NewSession {
    // …existing fields…
    user_id:   auth.map(|a| teramind_core::ids::UserId(a.user_id)),
    device_id: auth.map(|a| teramind_core::ids::DeviceId(a.device_id)),
};
```

4. Wrap the old private `route` as:

```rust
async fn route(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()> {
    let rd: RouteDeps = d.into();
    route_inner(&rd, env, None).await
}
```

- [ ] **Step 4: Re-export from teramindd lib**

In `crates/teramindd/src/lib.rs`, add:

```rust
pub use crate::services::ingest::{RouteDeps, IngestAuth, route_with_deps};
```

- [ ] **Step 5: Build the whole workspace**

Run: `cargo build --workspace`

Expected: success. (No new test yet — the daemon's existing tests cover the `route_inner` body via `route()`.)

- [ ] **Step 6: Run the daemon's full test suite to confirm zero regression**

Run: `cargo test -p teramindd`

Expected: all existing tests still PASS. (Plan H state: 168 tests.)

- [ ] **Step 7: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs \
        crates/teramindd/src/lib.rs \
        crates/teramind-db/src/repos/session.rs
git commit -m "refactor(daemon): extract route_inner + RouteDeps for server reuse"
```

---

## Section 16 — POST /v1/ingest

### Task 16.1: Failing test (auth-protected ingest E2E)

**Files:**
- Create: `crates/teramind-sync-server/tests/ingest_endpoint.rs`

- [ ] **Step 1: Test happy path + 2 failure modes**

```rust
//! E2E: redeem an invite, POST a batch, verify rows landed with annotation.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::config::*;
use teramind_sync_server::invite::InviteCode;
use teramind_sync_server::proof::{sign, ProofClaims, body_hash_hex, token_hash_hex};
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::{Duration as TDur, OffsetDateTime};

struct Redeemed {
    user_id: String,
    device_id: String,
    token: String,
    signing_key: SigningKey,
}

async fn boot() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(), database_url: "ignored".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    Ok((dir, sup, addr, pool))
}

async fn redeem(addr: SocketAddr, pool: &DbPool, email: &str) -> Redeemed {
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut OsRng);
    invites.create(&code.hash(), email, None, None,
                   OffsetDateTime::now_utc() + TDur::days(7)).await.unwrap();
    let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "dev1",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        })).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    Redeemed {
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        token: body["device_token"].as_str().unwrap().into(),
        signing_key: sk,
    }
}

fn signed(addr: SocketAddr, path: &str, body: &[u8], r: &Redeemed) -> reqwest::RequestBuilder {
    let url = format!("http://{addr}{path}");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "POST".into(), htu: url.clone(), iat: now,
        jti: format!("jti-{}", uuid::Uuid::new_v4()),
        ath: token_hash_hex(&r.token),
        bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &r.signing_key);
    reqwest::Client::new().post(&url)
        .header("Authorization", format!("Bearer {}", r.token))
        .header("X-Teramind-Proof", proof)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
}

fn sample_batch() -> serde_json::Value {
    let sid = uuid::Uuid::new_v4();
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    json!({
        "events": [
            { "client_event_id": uuid::Uuid::new_v4().to_string(),
              "ts": started.format(&time::format_description::well_known::Rfc3339).unwrap(),
              "event": {
                  "type": "session_start",
                  "session_id": sid.to_string(),
                  "agent_kind": "claude_code",
                  "cwd": "/repo",
                  "os": "linux", "hostname": "h", "user_login": "u",
                  "git_head": null, "git_branch": null,
                  "agent_session_id": null
              }
            }
        ]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ingest_with_valid_auth_lands_rows() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let r = redeem(addr, &pool, "alice@acme.dev").await;
    let body = serde_json::to_vec(&sample_batch())?;
    let resp = signed(addr, "/v1/ingest", &body, &r).send().await?;
    assert_eq!(resp.status(), 200);
    let summary: serde_json::Value = resp.json().await?;
    assert_eq!(summary["accepted"], 1);

    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sessions WHERE user_id = $1::uuid AND device_id = $2::uuid"
    ).bind(&r.user_id).bind(&r.device_id).fetch_one(pool.pg()).await?;
    assert_eq!(count, 1, "session row must be annotated with (user_id, device_id)");

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ingest_without_auth_is_401() -> anyhow::Result<()> {
    let (_d, sup, addr, _pool) = boot().await?;
    let resp = reqwest::Client::new().post(format!("http://{addr}/v1/ingest"))
        .json(&sample_batch()).send().await?;
    assert_eq!(resp.status(), 401);
    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ingest_idempotent_on_duplicate_client_event_id() -> anyhow::Result<()> {
    let (_d, sup, addr, _pool) = boot().await?;
    let r = redeem(addr, &_pool, "carol@acme.dev").await;
    let batch = sample_batch();
    let body = serde_json::to_vec(&batch)?;
    let first  = signed(addr, "/v1/ingest", &body, &r).send().await?;
    let second = signed(addr, "/v1/ingest", &body, &r).send().await?;
    assert_eq!(first.status(),  200);
    assert_eq!(second.status(), 200);
    let s: serde_json::Value = second.json().await?;
    // Idempotency: second submission produces accepted=0 (or duplicates=1).
    assert_eq!(s["accepted"].as_i64().unwrap() + s["duplicates"].as_i64().unwrap(), 1);
    sup.shutdown().await?; Ok(())
}
```

- [ ] **Step 2: Run, watch fail**

Run: `cargo test -p teramind-sync-server --test ingest_endpoint`

Expected: FAIL — `/v1/ingest` 404 (no route).

### Task 16.2: Implement /v1/ingest

**Files:**
- Modify: `crates/teramind-sync-server/src/handlers/ingest.rs`
- Modify: `crates/teramind-sync-server/src/server.rs`
- Modify: `crates/teramind-sync-server/src/state.rs`

- [ ] **Step 1: Extend AppState with a `RouteDeps` factory**

Server-side route dispatch needs `RouteDeps` (from teramindd). Construct it inside `AppState`:

```rust
// Append to state.rs
use teramindd::{RouteDeps, IngestAuth, route_with_deps};
use teramindd::services::session_manager::SessionManager;
use teramindd::services::write_tool_ring::WriteToolRing;
use teramindd::services::fs_watcher::WatchRegistry;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

impl AppState {
    pub fn route_deps(&self) -> RouteDeps {
        let raw_tx = tokio::sync::mpsc::unbounded_channel().0; // unused on server
        let gaps   = Arc::new(AtomicU64::new(0));
        RouteDeps {
            sessions: SessionManager::new(),
            agents: AgentRepo::new(self.pool.clone()),
            session_repo: SessionRepo::new(self.pool.clone()),
            trace: TraceRepo::new(self.pool.clone()),
            diffs: DiffRepo::new(self.pool.clone()),
            fs_registry: Arc::new(WatchRegistry::new(raw_tx, gaps)),
            write_tool_ring: WriteToolRing::new(64, time::Duration::milliseconds(2000)),
        }
    }
}
```

Note: server-side `WatchRegistry` and `WriteToolRing` are inert (no fs events arrive at the server). The constructors are zero-cost.

- [ ] **Step 2: Write the ingest handler**

```rust
//! POST /v1/ingest — receive a batch of EventEnvelopes from a remote daemon
//! and dispatch each through teramindd's reusable route_with_deps().

use crate::state::{AppState, AuthContext};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::{Deserialize, Serialize};
use teramind_core::types::ingest_event::EventEnvelope;
use teramindd::{IngestAuth, route_with_deps};

#[derive(Deserialize)]
pub struct IngestBatch {
    pub events: Vec<EventEnvelope>,
}

#[derive(Serialize, Default)]
pub struct IngestSummary {
    pub accepted: u32,
    pub duplicates: u32,
    pub rejected: Vec<RejectedEvent>,
}

#[derive(Serialize)]
pub struct RejectedEvent {
    pub client_event_id: String,
    pub reason: String,
}

pub async fn ingest(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(batch): Json<IngestBatch>,
) -> impl IntoResponse {
    if batch.events.len() > state.cfg.ingest.max_batch_size {
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(IngestSummary::default())).into_response();
    }
    let rd = state.route_deps();
    let ia = IngestAuth { user_id: auth.user_id.0, device_id: auth.device_id.0 };

    let mut summary = IngestSummary::default();
    for env in batch.events {
        let cid = env.client_event_id.0.to_string();
        match route_with_deps(&rd, env, Some(ia)).await {
            Ok(()) => summary.accepted += 1,
            Err(e) => {
                let s = e.to_string();
                if s.contains("duplicate key") || s.contains("unique constraint") {
                    summary.duplicates += 1;
                } else {
                    summary.rejected.push(RejectedEvent { client_event_id: cid, reason: s });
                }
            }
        }
    }
    (StatusCode::OK, Json(summary)).into_response()
}
```

- [ ] **Step 3: Wire the route + middleware**

Edit `crates/teramind-sync-server/src/server.rs`:

```rust
pub fn build_router(state: AppState) -> Router {
    use axum::routing::{get, post};
    let public = Router::new()
        .route("/v1/health",      get(handlers::health::health))
        .route("/v1/version",     get(handlers::health::version))
        .route("/v1/auth/redeem", post(handlers::redeem::redeem));
    let authed = Router::new()
        .route("/v1/ingest", post(handlers::ingest::ingest))
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::auth::auth_middleware));
    public.merge(authed).with_state(state).layer(TraceLayer::new_for_http())
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-sync-server --test ingest_endpoint -- --nocapture`

Expected: 3 PASS. (PG warmup ~15s each, batched runs ~30s total.)

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-sync-server/src/state.rs \
        crates/teramind-sync-server/src/handlers/ingest.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/ingest_endpoint.rs
git commit -m "feat(sync-server): POST /v1/ingest (auth + route reuse + annotation)"
```

---

## Section 17 — Admin: invite create / list / revoke

### Task 17.1: Implement subcommand bodies

**Files:**
- Modify: `crates/teramind-sync-server/src/admin.rs`

- [ ] **Step 1: Write the admin module**

Replace the stub with:

```rust
//! Admin subcommand bodies (invite, member).

use crate::config::ServerConfig;
use crate::invite::InviteCode;
use anyhow::Context;
use rand::rngs::OsRng;
use teramind_db::pool::DbPool;
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use teramind_core::ids::{DeviceId, InviteId, UserId};
use time::{Duration, OffsetDateTime};

pub struct AdminCtx { pub pool: DbPool, pub cfg: ServerConfig }

impl AdminCtx {
    pub async fn open(cfg: ServerConfig) -> anyhow::Result<Self> {
        let pool = DbPool::connect_url(&cfg.database_url).await?;
        Ok(Self { pool, cfg })
    }
}

pub async fn invite_create(
    ctx: &AdminCtx,
    email: &str,
    display_name: Option<&str>,
    created_by: Option<&str>,
    expires_in_days: Option<i64>,
) -> anyhow::Result<()> {
    let invites = InviteRepo::new(ctx.pool.clone());
    let days = expires_in_days.unwrap_or(ctx.cfg.auth.invite_default_expires_days);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(days);
    let code = InviteCode::generate(&mut OsRng);
    invites.create(&code.hash(), email, display_name, created_by, expires_at).await
        .context("create invite")?;
    println!("invite created:");
    println!("  code:    {}", code.as_str());
    println!("  email:   {email}");
    println!("  expires: {expires_at}");
    if let Some(by) = created_by { println!("  by:      {by}"); }
    Ok(())
}

pub async fn invite_list(ctx: &AdminCtx) -> anyhow::Result<()> {
    let invites = InviteRepo::new(ctx.pool.clone()).list_outstanding().await?;
    if invites.is_empty() { println!("no outstanding invites"); return Ok(()); }
    println!("{:<36}  {:<30}  {:<25}", "id", "email", "expires_at");
    for i in invites {
        println!("{:<36}  {:<30}  {}", i.id.0, i.invited_email, i.expires_at);
    }
    Ok(())
}

pub async fn invite_revoke(ctx: &AdminCtx, id_str: &str) -> anyhow::Result<()> {
    let id = InviteId(uuid::Uuid::parse_str(id_str).context("bad uuid")?);
    InviteRepo::new(ctx.pool.clone()).revoke(id).await?;
    println!("invite {id_str} revoked");
    Ok(())
}

pub async fn member_list(ctx: &AdminCtx) -> anyhow::Result<()> {
    let users = UserRepo::new(ctx.pool.clone()).list_all().await?;
    let devices = DeviceRepo::new(ctx.pool.clone());
    println!("{:<36}  {:<30}  {:>7}  {:<25}", "user_id", "email", "devices", "last_seen");
    for u in users {
        let ds = devices.list_for_user(u.id).await?;
        let last = ds.iter().filter_map(|d| d.last_seen_at).max();
        println!("{:<36}  {:<30}  {:>7}  {}",
                 u.id.0, u.email, ds.len(),
                 last.map(|t| t.to_string()).unwrap_or_else(|| "—".into()));
    }
    Ok(())
}

pub async fn member_revoke_device(ctx: &AdminCtx, id_str: &str) -> anyhow::Result<()> {
    let id = DeviceId(uuid::Uuid::parse_str(id_str).context("bad uuid")?);
    DeviceRepo::new(ctx.pool.clone()).revoke(id).await?;
    println!("device {id_str} revoked");
    Ok(())
}

pub async fn member_revoke_user(ctx: &AdminCtx, id_str: &str) -> anyhow::Result<()> {
    let id = UserId(uuid::Uuid::parse_str(id_str).context("bad uuid")?);
    UserRepo::new(ctx.pool.clone()).revoke(id).await?;
    println!("user {id_str} revoked (cascade: associated devices remain rows but auth lookups now fail)");
    Ok(())
}
```

### Task 17.2: Wire subcommands into the CLI

**Files:**
- Modify: `crates/teramind-sync-server/src/main.rs`

- [ ] **Step 1: Extend the `Cmd` enum**

Add inside the `Cmd` enum (after the existing variants):

```rust
    /// Manage invite codes.
    Invite {
        #[command(subcommand)]
        action: InviteAction,
    },
    /// Manage members + devices.
    Member {
        #[command(subcommand)]
        action: MemberAction,
    },
```

After the `Cmd` definition, add two enums:

```rust
#[derive(Subcommand)]
enum InviteAction {
    /// Create a new invite for an email.
    Create {
        #[arg(long)] email: String,
        #[arg(long)] name: Option<String>,
        #[arg(long)] created_by: Option<String>,
        #[arg(long)] expires_in_days: Option<i64>,
    },
    /// List outstanding invites.
    List,
    /// Revoke an invite by id.
    Revoke { id: String },
}

#[derive(Subcommand)]
enum MemberAction {
    /// List users + device counts.
    List,
    /// Revoke a single device by id.
    RevokeDevice { id: String },
    /// Revoke a user (cascade-revokes auth lookups for their devices).
    RevokeUser { id: String },
}
```

Then add the match arms in `main`:

```rust
Cmd::Invite { action } => {
    let cfg = teramind_sync_server::config::ServerConfig::load(&cli.config.unwrap_or_else(default_config_path))?;
    let ctx = teramind_sync_server::admin::AdminCtx::open(cfg).await?;
    match action {
        InviteAction::Create { email, name, created_by, expires_in_days } =>
            teramind_sync_server::admin::invite_create(&ctx, &email,
                name.as_deref(), created_by.as_deref(), expires_in_days).await,
        InviteAction::List => teramind_sync_server::admin::invite_list(&ctx).await,
        InviteAction::Revoke { id } => teramind_sync_server::admin::invite_revoke(&ctx, &id).await,
    }
}
Cmd::Member { action } => {
    let cfg = teramind_sync_server::config::ServerConfig::load(&cli.config.unwrap_or_else(default_config_path))?;
    let ctx = teramind_sync_server::admin::AdminCtx::open(cfg).await?;
    match action {
        MemberAction::List => teramind_sync_server::admin::member_list(&ctx).await,
        MemberAction::RevokeDevice { id } => teramind_sync_server::admin::member_revoke_device(&ctx, &id).await,
        MemberAction::RevokeUser   { id } => teramind_sync_server::admin::member_revoke_user(&ctx, &id).await,
    }
}
```

### Task 17.3: Sanity test the admin loop

**Files:**
- Create: `crates/teramind-sync-server/tests/admin_invite.rs`

- [ ] **Step 1: Write a test that exercises the admin module directly**

```rust
//! Black-box test of the admin module against an embedded PG.

use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::admin::{invite_create, invite_revoke, AdminCtx};
use teramind_sync_server::config::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_then_list_then_revoke() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cfg = ServerConfig {
        listen_addr: "x".into(), database_url: "x".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let ctx = AdminCtx { pool: pool.clone(), cfg };

    invite_create(&ctx, "alice@acme.dev", Some("Alice"), Some("admin"), Some(7)).await?;
    let outstanding = InviteRepo::new(pool.clone()).list_outstanding().await?;
    assert_eq!(outstanding.len(), 1);

    invite_revoke(&ctx, &outstanding[0].id.0.to_string()).await?;
    let outstanding = InviteRepo::new(pool.clone()).list_outstanding().await?;
    assert_eq!(outstanding.len(), 0, "revoked invite must drop from outstanding list");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-sync-server --test admin_invite -- --nocapture`

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/src/admin.rs \
        crates/teramind-sync-server/src/main.rs \
        crates/teramind-sync-server/tests/admin_invite.rs
git commit -m "feat(sync-server): admin invite + member subcommands"
```

---

## Section 18 — Admin smoke from the compiled binary

A small shell-level test that runs the actual `teramind-sync-server` binary, just to make sure clap wiring works end-to-end.

### Task 18.1: Binary smoke test

**Files:**
- Create: `crates/teramind-sync-server/tests/binary_smoke.rs`

- [ ] **Step 1: Write the test**

```rust
//! Confirms the compiled binary's CLI surface is wired up. Runs `version`
//! and `--help` against the binary itself.

use std::process::Command;

fn binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_teramind-sync-server"))
}

#[test]
fn version_subcommand_prints_version() {
    let out = Command::new(binary()).arg("version").output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("teramind-sync-server "));
}

#[test]
fn help_lists_subcommands() {
    let out = Command::new(binary()).arg("--help").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    for sub in &["serve", "migrate", "invite", "member", "version"] {
        assert!(s.contains(sub), "--help must list `{sub}`");
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-sync-server --test binary_smoke`

Expected: 2 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/tests/binary_smoke.rs
git commit -m "test(sync-server): binary smoke (version + help)"
```

---

## Section 19 — `teramind doctor` (client side): team-mode-aware health

(Foundation work; this surfaces "team mode not configured" in local-first installs so users discover the feature exists. The full team-mode rendering arrives in Plan J when team.toml/team-key actually exist.)

### Task 19.1: Extend doctor

**Files:**
- Modify: `crates/teramind/src/commands/doctor.rs`

- [ ] **Step 1: Locate the current doctor rendering**

Run: `grep -n 'fn render\|fn print_summary' crates/teramind/src/commands/doctor.rs | head -10`

Expected: identifies the function that prints summary provider lines (added in Plan H).

- [ ] **Step 2: Add a team-mode block**

Within the rendering function, after the existing summarizer block, add:

```rust
// team mode
let team_toml = paths.config_dir.join("team.toml");
if team_toml.exists() {
    println!("team mode:   configured (team.toml present at {})", team_toml.display());
    println!("             full team-mode health rendered in Plan J");
} else {
    println!("team mode:   not configured (run `teramind init --team --server=… --invite=…` to opt in)");
}
```

(`paths` is whatever `Paths` resolution the existing doctor code already uses; the exact variable name is in the surrounding function.)

- [ ] **Step 3: Run the doctor manually**

Run: `cargo run -p teramind -- doctor`

Expected: the new "team mode: not configured" line appears.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind/src/commands/doctor.rs
git commit -m "feat(cli): doctor surfaces team-mode opt-in pointer"
```

---

## Section 20 — TLS termination

axum-server with rustls. The server refuses to start without TLS unless `--insecure-allow-http` is passed.

### Task 20.1: Implement TLS serve

**Files:**
- Modify: `crates/teramind-sync-server/src/tls.rs`
- Modify: `crates/teramind-sync-server/src/server.rs`

- [ ] **Step 1: Write the TLS config loader**

Replace the stub of `tls.rs`:

```rust
//! Load rustls server config from PEM-encoded cert + key files.

use crate::config::TlsConfig;
use anyhow::{Context, anyhow};
use rustls_pemfile::Item;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

pub fn rustls_config(tls: &TlsConfig) -> anyhow::Result<Arc<RustlsServerConfig>> {
    let mut cert_reader = BufReader::new(File::open(&tls.cert_file)
        .with_context(|| format!("open cert {}", tls.cert_file.display()))?);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<_, _>>().context("parse cert PEM")?;
    if certs.is_empty() {
        return Err(anyhow!("no certificates found in {}", tls.cert_file.display()));
    }

    let mut key_reader = BufReader::new(File::open(&tls.key_file)
        .with_context(|| format!("open key {}", tls.key_file.display()))?);
    let key = rustls_pemfile::read_one(&mut key_reader)?
        .ok_or_else(|| anyhow!("no key in {}", tls.key_file.display()))?;
    let key: PrivateKeyDer<'static> = match key {
        Item::Pkcs8Key(k) => PrivateKeyDer::Pkcs8(k),
        Item::Pkcs1Key(k) => PrivateKeyDer::Pkcs1(k),
        Item::Sec1Key(k)  => PrivateKeyDer::Sec1(k),
        other => return Err(anyhow!("unsupported key type: {other:?}")),
    };

    let cfg = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(Arc::new(cfg))
}
```

- [ ] **Step 2: Replace the TLS serve stub**

In `crates/teramind-sync-server/src/server.rs`:

```rust
pub async fn serve_tls(
    state: AppState,
    addr: SocketAddr,
    tls: &crate::config::TlsConfig,
) -> anyhow::Result<()> {
    let app = build_router(state);
    let cfg = crate::tls::rustls_config(tls)?;
    let acceptor = axum_server::tls_rustls::RustlsConfig::from_config(cfg);
    info!(%addr, "teramind-sync-server listening (HTTPS)");
    axum_server::bind_rustls(addr, acceptor)
        .serve(app.into_make_service()).await?;
    Ok(())
}
```

- [ ] **Step 3: Build to confirm the dep wiring**

Run: `cargo build -p teramind-sync-server`

Expected: success.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-sync-server/src/tls.rs \
        crates/teramind-sync-server/src/server.rs
git commit -m "feat(sync-server): TLS termination via rustls"
```

---

## Section 21 — End-to-end team_harness integration test

A single test that combines redeem + ingest + DPoP-protected RPC in one flow.

### Task 21.1: Write the harness

**Files:**
- Create: `crates/teramind-sync-server/tests/team_harness.rs`

- [ ] **Step 1: Write the test**

```rust
//! End-to-end: spin up the server against an embedded PG, redeem an invite,
//! make several authenticated requests, verify rows landed correctly.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::config::*;
use teramind_sync_server::invite::InviteCode;
use teramind_sync_server::proof::{sign, ProofClaims, body_hash_hex, token_hash_hex};
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

struct Client {
    addr: SocketAddr,
    token: String,
    sk: SigningKey,
}

impl Client {
    async fn redeem(addr: SocketAddr, pool: &DbPool, email: &str) -> Self {
        let invites = InviteRepo::new(pool.clone());
        let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes().to_vec();
        let code = InviteCode::generate(&mut OsRng);
        invites.create(&code.hash(), email, None, None,
                       OffsetDateTime::now_utc() + TDur::days(7)).await.unwrap();
        let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
            .json(&json!({
                "invite_code": code.as_str(),
                "device_name": format!("{email}-dev"),
                "device_public_key_b64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD, &pk),
            })).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let body: serde_json::Value = r.json().await.unwrap();
        Self { addr, token: body["device_token"].as_str().unwrap().into(), sk }
    }

    fn signed_post(&self, path: &str, body: &[u8]) -> reqwest::RequestBuilder {
        let url = format!("http://{}{}", self.addr, path);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = ProofClaims {
            htm: "POST".into(), htu: url.clone(), iat: now,
            jti: format!("jti-{}", Uuid::new_v4()),
            ath: token_hash_hex(&self.token),
            bsh: body_hash_hex(body),
        };
        let proof = sign(&claims, &self.sk);
        reqwest::Client::new().post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Teramind-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body.to_vec())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_redeem_then_ingest_flow() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(), database_url: "ignored".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let alice = Client::redeem(addr, &pool, "alice@acme.dev").await;
    let bob   = Client::redeem(addr, &pool, "bob@acme.dev").await;

    // Each redeemer ships one SessionStart event.
    for client in [&alice, &bob] {
        let sid = Uuid::new_v4();
        let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let body = serde_json::to_vec(&json!({
            "events": [
                { "client_event_id": Uuid::new_v4().to_string(),
                  "ts": started.format(&time::format_description::well_known::Rfc3339).unwrap(),
                  "event": { "type": "session_start",
                             "session_id": sid.to_string(),
                             "agent_kind": "claude_code", "cwd": "/x",
                             "os": "linux", "hostname": "h", "user_login": "u",
                             "git_head": null, "git_branch": null, "agent_session_id": null } }
            ]
        }))?;
        let r = client.signed_post("/v1/ingest", &body).send().await?;
        assert_eq!(r.status(), 200);
    }

    // Each session is annotated with the redeeming user's id.
    let (total,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sessions WHERE user_id IS NOT NULL"
    ).fetch_one(pool.pg()).await?;
    assert_eq!(total, 2);

    let (alice_users,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sessions s \
         JOIN users u ON u.id = s.user_id \
         WHERE u.email = 'alice@acme.dev'"
    ).fetch_one(pool.pg()).await?;
    assert_eq!(alice_users, 1);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stolen_token_without_key_fails_403() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(), database_url: "ignored".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let alice = Client::redeem(addr, &pool, "alice@acme.dev").await;

    // Attacker has alice's token but their own signing key.
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let attacker_sk = SigningKey::from_bytes(&seed);
    let attacker = Client { addr, token: alice.token.clone(), sk: attacker_sk };

    let body = serde_json::to_vec(&json!({ "events": [] }))?;
    let r = attacker.signed_post("/v1/ingest", &body).send().await?;
    assert_eq!(r.status(), 403, "stolen token without matching private key must fail");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-sync-server --test team_harness -- --nocapture`

Expected: 2 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-sync-server/tests/team_harness.rs
git commit -m "test(sync-server): team harness E2E (redeem + ingest + DPoP theft)"
```

---

## Section 22 — Docker Compose for quick start

### Task 22.1: Dockerfile

**Files:**
- Create: `docker/sync-server/Dockerfile`

- [ ] **Step 1: Write the Dockerfile**

```dockerfile
# Build stage
FROM rust:1.93-slim AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p teramind-sync-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/teramind-sync-server /usr/local/bin/
RUN useradd -r -u 10001 teramind
USER teramind
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/teramind-sync-server"]
CMD ["serve"]
```

### Task 22.2: Compose file

**Files:**
- Create: `docker/sync-server/docker-compose.yml`

- [ ] **Step 1: Write compose**

```yaml
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: teramind
      POSTGRES_PASSWORD: changeme
      POSTGRES_DB: teramind
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "teramind"]
      interval: 5s
      timeout: 3s
      retries: 10

  sync-server:
    build:
      context: ../..
      dockerfile: docker/sync-server/Dockerfile
    environment:
      TERAMIND_SYNC_CONFIG: /etc/teramind-sync-server/config.toml
    volumes:
      - ./config.toml:/etc/teramind-sync-server/config.toml:ro
    depends_on:
      postgres:
        condition: service_healthy
    ports:
      - "8443:8443"
    command: ["serve", "--insecure-allow-http"]

volumes:
  pgdata:
```

### Task 22.3: Sample config

**Files:**
- Create: `docker/sync-server/config.toml`

- [ ] **Step 1: Write a sample config**

```toml
listen_addr  = "0.0.0.0:8443"
database_url = "postgres://teramind:changeme@postgres:5432/teramind"

# TLS omitted in the dev compose; we pass --insecure-allow-http on the command line.
# For production: provide [tls] cert_file/key_file and remove the insecure flag.

[auth]
invite_default_expires_days = 7
proof_replay_window_secs    = 60
proof_replay_cache_size     = 10000

[ingest]
max_batch_size         = 32
max_request_body_bytes = 10485760
```

- [ ] **Step 2: Commit (no test — manual smoke via the runbook)**

```bash
git add docker/sync-server
git commit -m "feat(sync-server): docker compose quick-start"
```

---

## Section 23 — Deployment runbook

### Task 23.1: Write the runbook

**Files:**
- Create: `docs/runbooks/sync-server-deploy.md`

- [ ] **Step 1: Write it**

```markdown
# Teramind Sync Server — Deployment Runbook

## Scope

How to deploy `teramind-sync-server` for a team. Covers the **single-binary**
mode (recommended for self-hosting) and the **docker-compose** mode (faster
to evaluate). Targets v1.0 — see the spec at `docs/superpowers/specs/2026-05-17-teramind-team-sync-design.md`
for the architectural picture.

## Prereqs

- Postgres 14+ with `pgcrypto` and `pg_trgm` extensions available. (pgvector
  becomes a hard requirement in Plan K when semantic search routes via the
  server; v1.0 of the sync server itself does not require pgvector yet.)
- A TLS certificate + key for the public hostname. Let's Encrypt via a
  reverse proxy is the typical path; the server can also terminate TLS
  directly via `[tls] cert_file = …, key_file = …`.
- A clock-synced host (`chronyd` / `timedatectl`). DPoP claims include `iat`;
  more than ±60 s skew between client and server rejects every request.

## Single-binary install

1. Build:
   ```bash
   cargo build --release -p teramind-sync-server
   ```

2. Copy `target/release/teramind-sync-server` to your server (eg. `/usr/local/bin/`).

3. Create the config:
   ```toml
   # /etc/teramind-sync-server/config.toml
   listen_addr  = "0.0.0.0:443"
   database_url = "postgres://teramind:REDACTED@127.0.0.1:5432/teramind"

   [tls]
   cert_file = "/etc/teramind/cert.pem"
   key_file  = "/etc/teramind/key.pem"
   ```

4. Run migrations:
   ```bash
   teramind-sync-server migrate
   ```

5. Start the service. A minimal systemd unit:

   ```ini
   [Unit]
   Description=Teramind Sync Server
   After=network-online.target postgresql.service
   Requires=postgresql.service

   [Service]
   ExecStart=/usr/local/bin/teramind-sync-server serve
   Environment=TERAMIND_SYNC_CONFIG=/etc/teramind-sync-server/config.toml
   User=teramind
   Restart=on-failure
   AmbientCapabilities=CAP_NET_BIND_SERVICE
   CapabilityBoundingSet=CAP_NET_BIND_SERVICE

   [Install]
   WantedBy=multi-user.target
   ```

6. Verify:
   ```bash
   curl -sk https://teramind.acme.dev/v1/health
   # {"status":"ok","db":"ok"}
   ```

## Docker Compose install (dev / evaluation)

```bash
cd docker/sync-server
docker compose up --build
# wait ~30s for build + migrations
curl http://localhost:8443/v1/health
```

The compose file uses `--insecure-allow-http` (no TLS). Do not use for
production — a real deployment terminates TLS either in the binary
(`[tls]`) or in a reverse proxy.

## Day-2: issuing invites

```bash
teramind-sync-server invite create \
    --email alice@acme.dev --name "Alice K." --created-by admin@acme.dev
# invite created:
#   code:    TM-…
#   email:   alice@acme.dev
#   expires: 2026-05-24T16:00:00Z
```

Hand the code to the developer; it's one-shot and expires in 7 days by
default. They run:

```bash
teramind init --team \
    --server https://teramind.acme.dev \
    --invite TM-…
```

(Plan J implements the `--team` flag end-to-end.)

## Day-2: revoking

- Lost laptop: `teramind-sync-server member revoke-device <device-id>`.
- Offboarding: `teramind-sync-server member revoke-user <user-id>`. (Devices
  remain rows but every auth lookup fails.)
- Unused invite that hasn't been redeemed: `teramind-sync-server invite revoke <invite-id>`.

## Observability

- The server emits JSON-formatted logs to stdout; pipe to your aggregator.
- `GET /v1/health` for liveness, `GET /v1/version` for build-id.
- Per-request tracing emits a `request_id`, the matched route, and the
  `(user_id, device_id)` if auth succeeded. Failed auth requests log the
  failure reason but not the bearer token.

## Backup

The DB is the source of truth. Standard Postgres backup applies. The server
itself is stateless beyond the in-memory DPoP replay cache, which is
ephemeral (a 60s replay window).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Every request 403 `invalid_proof` after a deploy | Wall clock skew | Run `timedatectl status`; fix NTP. |
| `teramind init --team` fails 410 | Invite expired | Re-issue. |
| `teramind init --team` fails 409 | Invite already redeemed | Re-issue (each device needs its own). |
| Server refuses to start: `TLS not configured` | No `[tls]` in config | Either configure TLS or pass `--insecure-allow-http`. |
```

- [ ] **Step 2: Commit**

```bash
git add docs/runbooks/sync-server-deploy.md
git commit -m "docs(runbook): sync server deployment"
```

---

## Section 24 — Final check

### Task 24.1: Run the entire workspace test suite

- [ ] **Step 1: Run everything**

Run: `cargo test --workspace`

Expected: every test passes. Plan H baseline is 168 tests. This plan adds approximately:

- §2: 1 db migration test
- §3-5: 3 + 3 + 3 = 9 repo tests
- §6: 6 invite-code tests
- §7: 3 token tests
- §8: 8 DPoP tests
- §9: 3 replay-cache tests
- §10: 2 config tests
- §12: 1 health smoke test
- §13: 5 auth-middleware tests
- §14: 4 redeem tests
- §16: 3 ingest tests
- §17: 1 admin test
- §18: 2 binary-smoke tests
- §21: 2 team-harness tests

Total new: ~52 tests. Expected total: ~220 PASS.

### Task 24.2: Run clippy + fmt

- [ ] **Step 1: Lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: zero warnings.

- [ ] **Step 2: Format check**

Run: `cargo fmt --all -- --check`

Expected: exit 0.

If any lint fires, fix it. If `fmt --check` is dirty, run `cargo fmt --all` and amend the relevant commit (only the touched commit; not a sweeping reformat).

### Task 24.3: Manual smoke against the compiled binary

- [ ] **Step 1: Start a dev compose stack**

Run:
```bash
cd docker/sync-server
docker compose up --build -d
sleep 20  # let PG warm up + migrations run
curl http://localhost:8443/v1/health
```

Expected: `{"status":"ok","db":"ok"}`.

- [ ] **Step 2: Issue and redeem an invite end-to-end**

```bash
docker compose exec sync-server teramind-sync-server invite create --email me@local
# captures the code from stdout
CODE=TM-…  # paste from above

# Construct a redeem payload by hand. (Plan J adds the `teramind init --team`
# flow that does this automatically.)
PK_B64=$(python3 -c 'import os,base64; print(base64.b64encode(os.urandom(32)).decode())')
curl -X POST http://localhost:8443/v1/auth/redeem \
    -H 'Content-Type: application/json' \
    -d "{\"invite_code\":\"$CODE\",\"device_name\":\"manual-smoke\",\"device_public_key_b64\":\"$PK_B64\"}"
```

Expected: `{"user_id":"…","device_id":"…","device_token":"tmd_v1_…","device_name":"manual-smoke"}`.

- [ ] **Step 3: Tear down**

```bash
docker compose down -v
```

### Task 24.4: Final commit + push

- [ ] **Step 1: Stage and commit anything stragglers (eg. clippy fixes)**

Run: `git status`

If anything is unstaged, stage + commit each logical chunk separately. Otherwise skip.

- [ ] **Step 2: Push the branch + open the PR**

```bash
git push -u origin feat/teramind-sync-server
gh pr create --title 'feat: teramind sync server (Plan I)' --body "$(cat <<'EOF'
## Summary

Lands the central `teramind-sync-server` binary that anchors team mode.

- Three new repos (`UserRepo`, `DeviceRepo`, `InviteRepo`) + additive migration.
- Invite-code redemption (`POST /v1/auth/redeem`) + long-lived bearer tokens.
- DPoP-style Ed25519 request signing; stolen bearer tokens alone fail 403.
- `POST /v1/ingest` reusing `teramindd::route_with_deps()` for handler parity.
- Admin CLI: `invite create|list|revoke`, `member list|revoke-device|revoke-user`.
- Health + version + TLS via rustls; Docker compose for quick starts.

Implements §1–§5 + §11–§12 of the team-sync spec.
Plans J/K/L follow (forwarder, MCP proxy, live propagation).

## Test plan

- [ ] `cargo test --workspace` is green (~220 tests).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is silent.
- [ ] `docker compose up` + `/v1/health` smokes.
- [ ] Manual redemption against the running container returns a token.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Spec coverage matrix

| Spec section | Plan I addresses | Plan I defers to |
|---|---|---|
| §1 Background / motivation | Implicit in scope | — |
| §2.1 In scope (v1.0) — server bin | §1, §12, §20 | — |
| §2.1 In scope (v1.0) — schema + repos | §2–§5 | — |
| §2.1 In scope (v1.0) — invite redemption + DPoP | §6–§9, §13–§14 | — |
| §2.1 In scope (v1.0) — /v1/ingest | §15–§16 | — |
| §2.1 In scope (v1.0) — admin CLI | §17–§18 | — |
| §2.1 In scope (v1.0) — TLS + docker | §20, §22 | — |
| §2.1 In scope (v1.0) — `teramind init --team` | — | Plan J |
| §2.1 In scope (v1.0) — JSONL forwarder + decision cache | — | Plan J |
| §2.1 In scope (v1.0) — MCP proxy + RPC | — | Plan K |
| §2.1 In scope (v1.0) — read-path fallback | — | Plan K |
| §2.1 In scope (v1.0) — WebSocket live events + `teramind feed` | — | Plan L |
| §3 High-level architecture | §1 (workspace), §11 (state), §12 (server) | — |
| §4.1 Workspace layout | §1, file structure table | Plans J/K/L add their own files |
| §4.2 Schema delta | §2 | — |
| §4.3 Existing service reuse — `route_with_deps` extracted | §15 | — |
| §5.1 Invite issuance | §17 | — |
| §5.2 Device redemption | §14 | Plan J adds the `teramind init --team` client |
| §5.3 Per-request auth (bearer + DPoP) | §6–§9, §13 | — |
| §5.4 Defense properties | §13 (proof_with_wrong_key_is_403), §21 (stolen_token_without_key_fails_403) | — |
| §5.5 Doctor surfaces | §19 (pointer line) | Plan J (full client-side rendering) |
| §6 Capture forwarding | §16 (server-side endpoint) | Plan J (client-side forwarder) |
| §7 MCP proxy | — | Plan K |
| §8 Live propagation | — | Plan L |
| §9 Configuration — server config.toml | §10 | — |
| §10 Testing — L1/L2 | §3–§18 | — |
| §10.3 L3 multi-process harness | §21 (server-side foundation) | Plans J/K/L for multi-daemon |
| §10.4 L4 nightly | — | Plans J/K/L combined |
| §10.5 L5 search-eval against server | — | Plan K |
| §11 Rollout — v1.0 milestone | Plans I–L collectively | — |
| §12 Glossary | — | — |
