# Teramind Live Events (Plan L) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Real-time visibility into team activity. The sync server (Plan I) maintains an in-process `broadcast` channel that publishers (ingest on `SessionEnd`, summarizer on `WikiPageReady`, skill-save handler on `SkillSaved`) push to. A new `GET /v1/events` WebSocket endpoint streams those events to subscribed daemons, gated by the existing DPoP auth at the upgrade handshake. A new `team_events` service inside each local daemon connects to the WebSocket and republishes events on a local broadcast bus. A new `teramind feed` CLI consumes that local bus and prints a human-readable activity log. Plus: the server now runs a periodic `traces_fts` materialized view refresh (the gap surfaced at end of Plan K).

**Architecture:** Server-side, a `tokio::sync::broadcast::Sender<TeamEvent>` lives in `AppState`. Plan I's `IngestService::route()` already runs on SessionEnd — we add a `bus.send(TeamEvent::SessionEnded { … })` call there. Plan H's `summarizer_worker` already calls `WikiRepo::upsert` on completion — we add a `bus.send(TeamEvent::WikiPageReady { … })`. Plan I's `SaveSkill` arm in `rpc_dispatch::dispatch` already returns a `SkillRef` — we add a `bus.send(TeamEvent::SkillSaved { … })` after the upsert. A new `GET /v1/events` axum WebSocket handler verifies the DPoP proof on the upgrade request, subscribes to the broadcast, and forwards each event as a JSON text frame. Client-side, `teramindd::services::team_events` connects with `tokio-tungstenite`, signs a DPoP proof, holds the connection, and republishes received events on a local `broadcast::Sender<TeamEvent>`. The new `teramind feed` CLI opens a local subscriber and prints `ts | user | event-kind | cwd | …` rows. The server also gets a 30-second `tokio::time::interval` task that runs `REFRESH MATERIALIZED VIEW CONCURRENTLY traces_fts`.

**Tech Stack:** Rust 1.93. New workspace dep: `tokio-tungstenite` 0.24. Reuses Plan J's DPoP signer + TeamConfig. Reuses Plan I's auth middleware (extracted slightly so it can verify the proof from a query string on the WS upgrade, since browsers can't easily set custom headers). Reuses `tokio::sync::broadcast` (already in tokio core).

---

## Spec coverage

This plan implements spec §8.1–§8.6 (live propagation) end-to-end and closes the §6.6 doctor surface gap (forwarder + events status). The `teramind feed` CLI is spec §8.4 (consumer #1). Coverage matrix at the bottom.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramind-core/src/team_event.rs` | Shared `TeamEvent` enum (SessionEnded, WikiPageReady, SkillSaved) |
| `crates/teramind-sync-server/src/handlers/events.rs` | `GET /v1/events` WebSocket upgrade + broadcast fan-out |
| `crates/teramind-sync-server/src/fts_refresh.rs` | Periodic `REFRESH MATERIALIZED VIEW traces_fts` task |
| `crates/teramindd/src/services/team_events.rs` | Local WebSocket subscriber + reconnect + local rebroadcast |
| `crates/teramind/src/commands/feed.rs` | `teramind feed [--follow]` CLI body |
| `crates/teramind-sync-server/tests/events_endpoint.rs` | Server-side WebSocket end-to-end |
| `crates/teramindd/tests/team_events_e2e.rs` | Daemon-side reconnect + republish |
| `crates/teramind/tests/feed_cli.rs` | `teramind feed` smoke |

**Modified files:**

- `Cargo.toml` (workspace) — add `tokio-tungstenite = { version = "0.24", default-features = false, features = ["rustls-tls-webpki-roots"] }` and `axum`'s `ws` feature.
- `crates/teramind-sync-server/Cargo.toml` — enable `axum`'s `ws` feature; depend on `tokio-tungstenite` for tests.
- `crates/teramind-sync-server/src/state.rs` — `AppState` gains `pub bus: tokio::sync::broadcast::Sender<TeamEvent>`. Capacity 256.
- `crates/teramind-sync-server/src/server.rs` — register `/v1/events` (no auth middleware — the WS handler verifies proof itself via query-string param).
- `crates/teramind-sync-server/src/main.rs` (or `app.rs` equivalent) — spawn the `fts_refresh` task.
- `crates/teramindd/src/services/rpc_dispatch.rs` — after a successful `Request::SaveSkill`, `bus.send(TeamEvent::SkillSaved { … })` IF the server-side dispatch has access to a bus. Pipe an `Option<broadcast::Sender<TeamEvent>>` through the dispatch's `RpcDeps` or `AuthContext`.
- `crates/teramind-sync-server/src/handlers/ingest.rs` — after each successfully-routed event, if the event was `SessionEnd`, emit `TeamEvent::SessionEnded`. Same for `WikiPageReady` if the server eventually runs summarizer_worker (deferred — wiki publish stays in summarizer_worker on the daemon side for v1.0; the server doesn't run summarizer today). Document this and emit only what is feasible now.
- `crates/teramindd/src/services/mod.rs` — register `team_events`.
- `crates/teramindd/src/app.rs` — spawn `team_events` if team mode is configured.
- `crates/teramind/src/cli.rs` — add `Feed { --follow }` subcommand.
- `crates/teramind/src/commands/mod.rs` — register `feed`.
- `crates/teramind/src/commands/doctor.rs` — add the `team events: ws connected for …` line when team mode is on.

---

## Section 0 — Pre-flight

### Task 0.1: Branch from a green main

- [ ] Run:
```bash
git checkout main
cargo build --workspace
git checkout -b feat/teramind-live-events
git log --oneline -5
```

Expect: build silent; HEAD on new branch. Plan K merge commit in recent history.

---

## Section 1 — `TeamEvent` enum

### Task 1.1: Define the enum

**File:** `crates/teramind-core/src/team_event.rs`

```rust
//! Live-propagation events. Server-side publishers (ingest, summarizer,
//! save_skill) `bus.send(...)` one of these; subscribed daemons receive
//! them via the /v1/events WebSocket.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TeamEvent {
    SessionEnded {
        session_id: Uuid,
        user_id: Uuid,
        cwd: String,
        #[serde(with = "time::serde::rfc3339")]
        ts: time::OffsetDateTime,
    },
    WikiPageReady {
        page_id: Uuid,
        session_id: Uuid,
        user_id: Uuid,
        cwd: String,
        title: String,
        #[serde(with = "time::serde::rfc3339")]
        ts: time::OffsetDateTime,
    },
    SkillSaved {
        skill_id: Uuid,
        user_id: Uuid,
        name: String,
        #[serde(with = "time::serde::rfc3339")]
        ts: time::OffsetDateTime,
    },
}

impl TeamEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            TeamEvent::SessionEnded { .. } => "session_ended",
            TeamEvent::WikiPageReady { .. } => "wiki_page_ready",
            TeamEvent::SkillSaved { .. } => "skill_saved",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let evt = TeamEvent::SessionEnded {
            session_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            cwd: "/repo".into(),
            ts: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        };
        let j = serde_json::to_string(&evt).unwrap();
        let parsed: TeamEvent = serde_json::from_str(&j).unwrap();
        match parsed {
            TeamEvent::SessionEnded { cwd, .. } => assert_eq!(cwd, "/repo"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
```

Register: `pub mod team_event;` in `crates/teramind-core/src/lib.rs`.

### Task 1.2: Verify + commit

```bash
cargo test -p teramind-core team_event::
cargo clippy -p teramind-core --all-targets -- -D warnings
git add crates/teramind-core/src/team_event.rs crates/teramind-core/src/lib.rs
git commit -m "feat(core): TeamEvent enum"
```

---

## Section 2 — Server-side broadcast bus

### Task 2.1: Add the bus to AppState

**File:** `crates/teramind-sync-server/src/state.rs`

Add:

```rust
pub bus: tokio::sync::broadcast::Sender<teramind_core::team_event::TeamEvent>,
```

In `AppState::new`:

```rust
let (bus, _rx) = tokio::sync::broadcast::channel(256);
Self {
    // …existing fields…
    bus,
}
```

(The `_rx` is dropped — subscribers grab their own via `bus.subscribe()` later. The Sender alone keeps the channel alive because it's stored in `AppState`.)

### Task 2.2: Publish on ingest

**File:** `crates/teramind-sync-server/src/handlers/ingest.rs`

After the existing batch dispatch loop, walk the batch a second time (cheap — the events are still in memory) and publish per event-type. Better: pipe the bus through to where each event is dispatched so we publish exactly when route_with_deps succeeds.

Easiest implementation: after `route_with_deps` returns `Ok(())`, inspect the original `env` for `IngestEvent::SessionEnd { session_id, .. }` and call `state.bus.send(TeamEvent::SessionEnded { … })`. Skip publish on `WikiPageReady` for v1.0 (the summarizer doesn't run server-side yet).

Pseudo-code outline (adapt to the actual `ingest::ingest` handler shape):

```rust
for env in batch.events {
    let cid = env.client_event_id.0.to_string();
    let event_for_publish = env.event.clone();
    match route_with_deps(&rd, env, Some(ia)).await {
        Ok(()) => {
            summary.accepted += 1;
            publish_on_success(&state, &event_for_publish, ia.user_id).await;
        }
        Err(e) => { /* existing branch */ }
    }
}

async fn publish_on_success(
    state: &AppState,
    event: &teramind_core::types::ingest_event::IngestEvent,
    user_id: uuid::Uuid,
) {
    use teramind_core::types::ingest_event::IngestEvent;
    if let IngestEvent::SessionEnd { session_id, cwd, ended_at, .. } = event {
        // SessionEnd's exact field shape may differ — inspect.
        let _ = state.bus.send(teramind_core::team_event::TeamEvent::SessionEnded {
            session_id: session_id.0,
            user_id,
            cwd: cwd.clone(),
            ts: *ended_at,
        });
    }
}
```

**Inspect first:** the `IngestEvent::SessionEnd` variant's actual fields. The plan above assumes `{ session_id, cwd, ended_at, … }`. Adapt as needed; the `cwd` may come from looking up the row in `sessions` after insertion if it's not in the SessionEnd event itself. If you'd need a DB lookup just to populate cwd, defer that — send a minimal event with only `session_id, user_id, ts` and update spec coverage notes accordingly.

### Task 2.3: Publish on save_skill

**File:** `crates/teramindd/src/services/rpc_dispatch.rs`

Wire the bus through to `dispatch`. Two ways:
1. Add `Option<broadcast::Sender<TeamEvent>>` to `RpcDeps`. Daemon-side `RpcDeps` builds with `None`; server-side `AppState::rpc_deps()` builds with `Some(state.bus.clone())`.
2. Pass the bus as an additional parameter to `dispatch(...)`.

Option 1 keeps the signature stable. Pick that.

Add to `RpcDeps`:
```rust
pub event_bus: Option<tokio::sync::broadcast::Sender<teramind_core::team_event::TeamEvent>>,
```

In the `Request::SaveSkill` arm, after the upsert returns Ok:
```rust
if let Some(bus) = deps.event_bus.as_ref() {
    let user_id = auth.map(|a| a.user_id).unwrap_or_default();
    let _ = bus.send(teramind_core::team_event::TeamEvent::SkillSaved {
        skill_id: id.0, user_id, name: r.name.clone(),
        ts: time::OffsetDateTime::now_utc(),
    });
}
```

Update the daemon-side `RpcDeps` construction (in `ipc_server::DaemonIpcHandler::rpc_deps()`) to set `event_bus: None`. Update `AppState::rpc_deps()` to set `event_bus: Some(self.bus.clone())`.

### Task 2.4: Verify

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind-sync-server -- --test-threads=1
cargo test -p teramindd -- --test-threads=1
```

Existing tests must keep passing.

### Task 2.5: Commit

```bash
git add crates/teramind-sync-server/src/state.rs \
        crates/teramind-sync-server/src/handlers/ingest.rs \
        crates/teramindd/src/services/rpc_dispatch.rs \
        crates/teramindd/src/services/ipc_server.rs
git commit -m "feat(sync-server): broadcast bus + publish on session_end + save_skill"
```

---

## Section 3 — `GET /v1/events` WebSocket

### Task 3.1: Failing test (round-trip)

**File:** `crates/teramind-sync-server/tests/events_endpoint.rs`

Use `tokio-tungstenite` as a test client. Add to `crates/teramind-sync-server/Cargo.toml` `[dev-dependencies]`:

```toml
tokio-tungstenite = { workspace = true }
futures-util = "0.3"
```

Workspace `Cargo.toml`:

```toml
tokio-tungstenite = { version = "0.24", default-features = false, features = ["rustls-tls-webpki-roots"] }
```

Test sketch:

```rust
//! Server-side /v1/events WebSocket: redeem an invite, open a WS connection
//! with a DPoP proof in the `?proof=` query param, then ingest a SessionEnd
//! event; assert the WS subscriber receives a TeamEvent::SessionEnded frame.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use std::time::Duration;
use teramind_core::team_event::TeamEvent;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;
use futures_util::{SinkExt, StreamExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_end_event_streams_to_ws_subscriber() -> anyhow::Result<()> {
    // boot + redeem (copy helpers from rpc_endpoint.rs)
    // open WS with proof query
    // POST /v1/ingest a SessionEnd event
    // assert WS frame arrives within 1s with TeamEvent::SessionEnded
    todo!("write the full test body; placeholder elided for plan brevity")
}
```

Fill in the boot/redeem helpers from `tests/rpc_endpoint.rs` (Plan K). The WS handshake must:
1. Build a DPoP proof for `GET https://<host>/v1/events` (htm="GET").
2. Pass `?proof=<base64url-of-proof>` as a query string param (browsers can't easily set custom headers on WS upgrade, so we use a query param as a workaround for v1.0; spec §8.2 hints at this).
3. Send the bearer token in the `Authorization` header (tungstenite supports this).
4. After upgrade, `socket.next().await` gives a `Ok(Message::Text(json))` matching `TeamEvent::SessionEnded`.

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramind-sync-server --test events_endpoint -- --test-threads=1`. Expected: FAIL (route not implemented).

### Task 3.2: Implement the WS handler

**File:** `crates/teramind-sync-server/src/handlers/events.rs`

```rust
//! GET /v1/events WebSocket. Verifies the DPoP proof at upgrade time, then
//! streams TeamEvents from the AppState broadcast bus.

use crate::proof::{verify, ProofError};
use crate::state::AppState;
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Query, State},
    http::{header, StatusCode}, response::IntoResponse,
};
use base64::Engine;
use serde::Deserialize;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio::sync::broadcast;

#[derive(Deserialize)]
pub struct EventsQuery {
    /// base64url-encoded DPoP proof. Required.
    pub proof: String,
}

pub async fn events(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    // 1. Parse Authorization.
    let bearer = headers.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_string();
    let token = crate::token::DeviceToken::parse(&bearer).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let device = state.devices.get_active_by_token_hash(&token.hash()).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // 2. Decode the proof from the query string.
    let proof_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&q.proof).map_err(|_| StatusCode::FORBIDDEN)?;
    let proof_str = std::str::from_utf8(&proof_bytes).map_err(|_| StatusCode::FORBIDDEN)?;

    // 3. Verify the proof. Body hash is sha256(b"") because GET has no body.
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let scheme = if headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https") { "https" } else { "http" };
    let url = format!("{scheme}://{host}/v1/events");
    let body_hash = teramind_core::dpop::body_hash_hex(b"");
    let token_hash = teramind_core::dpop::token_hash_hex(token.as_str());

    let claims = verify(proof_str, &device.public_key, "GET", &url,
                        &body_hash, &token_hash, now,
                        state.cfg.auth.proof_replay_window_secs)
        .map_err(|e: ProofError| {
            tracing::warn!(error = %e, "events WS proof verify failed");
            StatusCode::FORBIDDEN
        })?;

    if !state.replay.check_and_insert(device.id, &claims.jti) {
        return Err(StatusCode::FORBIDDEN);
    }

    // 4. Upgrade and fan out events.
    let rx = state.bus.subscribe();
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, rx)))
}

async fn handle_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<TeamEvent>,
) {
    // Send a hello frame.
    let hello = serde_json::json!({
        "type": "hello",
        "server_version": teramind_sync_server::VERSION,
    });
    if socket.send(Message::Text(hello.to_string())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            // Forward bus events to the socket.
            msg = rx.recv() => {
                match msg {
                    Ok(evt) => {
                        let json = match serde_json::to_string(&evt) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(json)).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Client too slow — close the connection; it can reconnect.
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            // Handle client frames (mostly ping/pong/close).
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            return;
                        }
                    }
                    Some(Err(_)) => return,
                    _ => {} // ignore text/binary client→server frames
                }
            }
        }
    }
}
```

Add `pub mod events;` to `crates/teramind-sync-server/src/handlers/mod.rs`. Add the route in `server.rs::build_router` to the **public** sub-router (the auth middleware would consume the request body, but WebSocket upgrade needs to keep it):

```rust
.route("/v1/events", axum::routing::get(handlers::events::events))
```

Enable axum's `ws` feature in `crates/teramind-sync-server/Cargo.toml`:

```toml
axum = { workspace = true, features = ["ws"] }
```

(If the workspace declaration of axum doesn't already include `ws`, prefer extending crate-level features over reshaping the workspace dep.)

### Task 3.3: Verify + commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind-sync-server --test events_endpoint -- --test-threads=1
git add Cargo.toml \
        crates/teramind-sync-server/Cargo.toml \
        crates/teramind-sync-server/src/handlers/events.rs \
        crates/teramind-sync-server/src/handlers/mod.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/events_endpoint.rs
git commit -m "feat(sync-server): GET /v1/events WebSocket fan-out"
```

---

## Section 4 — Periodic `traces_fts` refresh

(Closes the gap surfaced at end of Plan K.)

### Task 4.1: Refresh task

**File:** `crates/teramind-sync-server/src/fts_refresh.rs`

```rust
//! Periodic refresh of the traces_fts materialized view.

use std::sync::Arc;
use std::time::Duration;
use teramind_db::pool::DbPool;
use tracing::{info, warn};

pub fn spawn(pool: DbPool, interval: Duration) -> Arc<tokio::task::JoinHandle<()>> {
    Arc::new(tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // first tick fires immediately; let one period elapse
        loop {
            tick.tick().await;
            match sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY traces_fts")
                .execute(pool.pg()).await
            {
                Ok(_) => info!("traces_fts refreshed"),
                Err(e) => warn!(error = %e, "traces_fts refresh failed"),
            }
        }
    }))
}
```

Register `pub mod fts_refresh;` in `crates/teramind-sync-server/src/lib.rs`.

In `crates/teramind-sync-server/src/main.rs` (the `Serve` arm), after `migrate::run`, spawn the task:

```rust
let _fts_refresh = teramind_sync_server::fts_refresh::spawn(pool.clone(), std::time::Duration::from_secs(30));
```

### Task 4.2: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-sync-server/src/fts_refresh.rs \
        crates/teramind-sync-server/src/lib.rs \
        crates/teramind-sync-server/src/main.rs
git commit -m "feat(sync-server): periodic traces_fts refresh"
```

(No new test — the two-dev E2E in Plan K already exercises FTS implicitly, just with a manual refresh that's now optional.)

---

## Section 5 — Daemon-side `team_events` subscriber

### Task 5.1: Failing test

**File:** `crates/teramindd/tests/team_events_e2e.rs`

```rust
//! E2E: spin up server, redeem an invite, start the team_events subscriber
//! in the daemon, publish a TeamEvent on the server's bus, assert the local
//! broadcast receives a matching event.

// Same shape as Plan K's two_dev_team_mode.rs:
// 1. boot sync server
// 2. redeem alice
// 3. start TeamEvents::spawn pointing at the server, with a TeamConfig + signing key
// 4. on the server side, send a TeamEvent on the bus directly (bypass /v1/ingest)
// 5. wait for the daemon's local broadcast to receive it
// 6. assert
```

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramindd --test team_events_e2e -- --test-threads=1` → FAIL (TeamEvents not yet).

### Task 5.2: Implement the subscriber

**File:** `crates/teramindd/src/services/team_events.rs`

```rust
//! Subscribes to the server's /v1/events WebSocket and republishes received
//! events on a local broadcast bus. Reconnects with exponential backoff.

use anyhow::{Context, Result};
use base64::Engine;
use ed25519_dalek::SigningKey;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::team::TeamConfig;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};
use tracing::{info, warn};
use futures_util::StreamExt;

pub struct TeamEventsDeps {
    pub team_cfg: Arc<TeamConfig>,
    pub signing_key: Arc<SigningKey>,
    pub local_bus: broadcast::Sender<TeamEvent>,
}

pub struct TeamEvents {
    _handle: tokio::task::JoinHandle<()>,
}

impl TeamEvents {
    pub fn spawn(deps: TeamEventsDeps) -> Self {
        let handle = tokio::spawn(async move { run_loop(deps).await; });
        Self { _handle: handle }
    }
}

async fn run_loop(deps: TeamEventsDeps) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect_and_pump(&deps).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => warn!(error = %e, ?backoff, "team_events connect failed; reconnecting"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn connect_and_pump(deps: &TeamEventsDeps) -> Result<()> {
    // Build proof.
    let server = deps.team_cfg.server_url.replace("http://", "ws://").replace("https://", "wss://");
    let url_for_signing = format!("{}/v1/events", deps.team_cfg.server_url);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "GET".into(), htu: url_for_signing.clone(), iat: now,
        jti: format!("jti-{}", uuid::Uuid::new_v4()),
        ath: token_hash_hex(&deps.team_cfg.device_token),
        bsh: body_hash_hex(b""),
    };
    let proof = sign(&claims, &deps.signing_key);
    let proof_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof.as_bytes());

    let ws_url = format!("{server}/v1/events?proof={proof_b64}");
    let req = Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {}", deps.team_cfg.device_token))
        .header("Host", deps.team_cfg.server_url.trim_start_matches("http://").trim_start_matches("https://"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(()).context("build ws request")?;

    let (ws, _resp) = tokio_tungstenite::connect_async(req).await
        .context("ws connect")?;
    info!(%ws_url, "team_events connected");
    let (_w, mut r) = ws.split();
    while let Some(msg) = r.next().await {
        let msg = msg.context("ws recv")?;
        if let Message::Text(text) = msg {
            // Discard "hello" frames; route TeamEvents.
            if let Ok(evt) = serde_json::from_str::<TeamEvent>(&text) {
                let _ = deps.local_bus.send(evt);
            }
        }
    }
    Ok(())
}
```

Add to `crates/teramindd/Cargo.toml`:
```toml
tokio-tungstenite = { workspace = true }
futures-util = "0.3"
```

Register `pub mod team_events;` in `crates/teramindd/src/services/mod.rs`.

### Task 5.3: Wire spawn in app.rs

In `crates/teramindd/src/app.rs`, after Plan J's `team_sync` spawn:

```rust
let local_bus = tokio::sync::broadcast::Sender::<teramind_core::team_event::TeamEvent>::new(256).0;
// Actually:
let (local_bus_tx, _local_bus_rx) = tokio::sync::broadcast::channel::<teramind_core::team_event::TeamEvent>(256);
let _team_events = team_mode.as_ref().map(|(cfg, sk)| {
    crate::services::team_events::TeamEvents::spawn(
        crate::services::team_events::TeamEventsDeps {
            team_cfg: cfg.clone(),
            signing_key: sk.clone(),
            local_bus: local_bus_tx.clone(),
        }
    )
});
```

Stash `local_bus_tx` in the daemon's state so `teramind feed` (which talks to the daemon via IPC) can subscribe — but for v1.0 the simpler path is to print events from the same process. We defer the IPC-mediated subscription path to v1.1 and make `teramind feed` open its own WebSocket directly (see §6).

### Task 5.4: Verify

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramindd --test team_events_e2e -- --test-threads=1
```

Expected: PASS.

### Task 5.5: Commit

```bash
git add crates/teramindd/Cargo.toml \
        crates/teramindd/src/services/team_events.rs \
        crates/teramindd/src/services/mod.rs \
        crates/teramindd/src/app.rs \
        crates/teramindd/tests/team_events_e2e.rs
git commit -m "feat(daemon): team_events WebSocket subscriber"
```

---

## Section 6 — `teramind feed` CLI

The `feed` subcommand opens its own WebSocket connection to the server (same auth as the daemon does) and prints a row per event. This avoids needing a new IPC mechanism for daemon → CLI streaming.

### Task 6.1: Add subcommand

**File:** `crates/teramind/src/cli.rs`

Add to the `Cmd` enum:

```rust
/// Stream live team activity (WebSocket; requires team mode).
Feed {
    /// Keep streaming until interrupted.
    #[arg(long)]
    follow: bool,
    /// Print recent buffered events before tailing (best-effort; v1.0 prints nothing).
    #[arg(long, default_value = "0")]
    backlog: u32,
},
```

### Task 6.2: Implement

**File:** `crates/teramind/src/commands/feed.rs`

```rust
//! `teramind feed [--follow]` — open the sync server's /v1/events WS and
//! print one human-readable row per TeamEvent.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use ed25519_dalek::SigningKey;
use std::sync::Arc;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::team::TeamConfig;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};
use futures_util::StreamExt;

pub async fn run(follow: bool, _backlog: u32) -> Result<()> {
    let cfg_dir = teramind_core::team::default_config_dir();
    let cfg = TeamConfig::load(&cfg_dir.join("team.toml"))
        .with_context(|| format!("load {}/team.toml — team mode required", cfg_dir.display()))?;
    let key = Arc::new(teramind_core::team::load_signing_key(&cfg_dir.join("team-key"))?);

    let server = cfg.server_url.replace("http://", "ws://").replace("https://", "wss://");
    let url_for_signing = format!("{}/v1/events", cfg.server_url);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "GET".into(), htu: url_for_signing.clone(), iat: now,
        jti: format!("jti-{}", uuid::Uuid::new_v4()),
        ath: token_hash_hex(&cfg.device_token),
        bsh: body_hash_hex(b""),
    };
    let proof = sign(&claims, &key);
    let proof_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof.as_bytes());
    let ws_url = format!("{server}/v1/events?proof={proof_b64}");

    let req = Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {}", cfg.device_token))
        .header("Host", cfg.server_url.trim_start_matches("http://").trim_start_matches("https://"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())?;

    let (ws, _) = tokio_tungstenite::connect_async(req).await
        .context("connect to /v1/events — is the server up + team.toml current?")?;
    let (_w, mut r) = ws.split();

    println!("{:<25} {:<15} {:<60}", "ts", "kind", "details");
    while let Some(msg) = r.next().await {
        match msg? {
            Message::Text(txt) => {
                if let Ok(evt) = serde_json::from_str::<TeamEvent>(&txt) {
                    print_event(&evt);
                    if !follow { return Ok(()); }
                }
                // Skip non-TeamEvent frames (e.g. hello).
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
    Err(anyhow!("server closed the connection"))
}

fn print_event(evt: &TeamEvent) {
    match evt {
        TeamEvent::SessionEnded { ts, cwd, .. } =>
            println!("{:<25} {:<15} {}", ts, "session_ended", cwd),
        TeamEvent::WikiPageReady { ts, title, cwd, .. } =>
            println!("{:<25} {:<15} {} — {}", ts, "wiki_page_ready", cwd, title),
        TeamEvent::SkillSaved { ts, name, .. } =>
            println!("{:<25} {:<15} {}", ts, "skill_saved", name),
    }
}
```

Add `pub mod feed;` to `crates/teramind/src/commands/mod.rs`. Dispatch in `main.rs`:

```rust
Cmd::Feed { follow, backlog } => commands::feed::run(follow, backlog).await,
```

Add to `crates/teramind/Cargo.toml` `[dependencies]`:

```toml
tokio-tungstenite = { workspace = true }
futures-util = "0.3"
base64 = { workspace = true }
ed25519-dalek = { workspace = true }
teramind-core = { path = "../teramind-core" }  # already there
```

### Task 6.3: Smoke test

**File:** `crates/teramind/tests/feed_cli.rs`

```rust
//! Smoke test for `teramind feed`: spin up a server, write a fake team.toml,
//! emit one event server-side, and assert the CLI prints a line containing
//! the expected kind.

// Pattern: launch teramind-sync-server as a subprocess OR construct an axum
// server inline (same as Plan I/J/K tests). Redeem an invite via reqwest.
// Set XDG_CONFIG_HOME to a tempdir; copy the team.toml + team-key the redeem
// produced into <tempdir>/teramind/. Spawn `teramind feed` (no --follow)
// as a subprocess and assert it prints "session_ended" after we trigger the
// bus.send.

// Full body elided for brevity — match the shape of Plan J's init_team test.
```

This test is fiddly (subprocess + env var + temp dir). If it becomes a time sink, downgrade it to a lib-style call into `teramind::commands::feed::run` rather than a real subprocess invocation, with the test driving the server's bus directly.

### Task 6.4: Doctor surface

In `crates/teramind/src/commands/doctor.rs`, after the existing team-mode block from Plan J §13, add:

```rust
// team events status — best-effort; v1.0 just reports whether the local
// daemon has team_events configured. Full WS connection status comes via
// IPC StatusReport in v1.1.
if team_toml.exists() {
    println!("team events: wired (run `teramind feed` to subscribe)");
}
```

### Task 6.5: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind --test feed_cli -- --test-threads=1
git add crates/teramind/Cargo.toml \
        crates/teramind/src/cli.rs \
        crates/teramind/src/commands/feed.rs \
        crates/teramind/src/commands/mod.rs \
        crates/teramind/src/commands/doctor.rs \
        crates/teramind/src/main.rs \
        crates/teramind/tests/feed_cli.rs
git commit -m "feat(cli): teramind feed (WebSocket subscriber)"
```

---

## Section 7 — Final check

### Task 7.1: Workspace sweep

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plan K baseline: 326 tests. Plan L adds approximately:
- §1 team_event: 1
- §3 events_endpoint: 1
- §5 team_events_e2e: 1
- §6 feed_cli: 1 (or skipped if subprocess test is too fiddly)

Expected total: ~330.

### Task 7.2: Report

Print HEAD SHA, total commits from main, total tests, any failures.

Do NOT push or open a PR.

---

## Spec coverage matrix

| Spec section | Plan L addresses | Notes |
|---|---|---|
| §2.1 In-scope — live propagation v1.0 | §1–§6 | — |
| §2.1 In-scope — `teramind feed` | §6 | — |
| §6.6 Doctor forwarder surfaces | §6 (doctor adds team-events line) | Full throughput stats deferred to v1.1 |
| §8.1 TeamEvent variants | §1 | — |
| §8.2 /v1/events with proof handshake | §3 | DPoP proof in `?proof=` query param (browsers can't easily set custom headers on WS upgrade) |
| §8.3 Local-side subscription + reconnect | §5 | Exponential backoff 1s → 60s |
| §8.4 `teramind feed` | §6 | — |
| §8.5 SessionStart auto-recall freshness | — | Deferred — needs daemon to track a marker file per cwd; v1.1 |
| §8.6 Doctor surfaces | §6 (partial) | Full ws-uptime in v1.1 |
| §10.3 L3 multi-process harness | §5 | The daemon-side E2E exercises the full server→ws→local-bus path |

---
