# Teramind MCP Proxy (Plan K) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `mcp__teramind__{search,recall,wiki,save_skill}` and the SessionStart auto-recall hook work seamlessly in **either** local-first or team mode. Same agent-facing tool names, same JSON shapes; only the transport changes underneath. Server-side adds a `POST /v1/rpc` endpoint that reuses the daemon's dispatch logic with an `AuthContext`. When the server is unreachable, *read* tools fall back to grep over local JSONL with `degraded: true`; *write* tools fail loudly.

**Architecture:** Refactor `teramindd::services::ipc_server::handle_request` so its body lives in a free function `dispatch(deps: &RpcDeps, req: Request, auth: Option<AuthContext>) -> Response` (parallel to Plan I's `route_with_deps`). The server builds a `POST /v1/rpc` handler that wraps the same dispatch with `auth = Some(...)` from the bearer/DPoP middleware. Both the MCP server (`teramind-mcp`) and the hook (`teramind-hook`) switch from a hard-coded `IpcClient` to an `RpcTransport` trait. Two impls: `LocalIpcTransport` (the existing UDS/named-pipe path) and `HttpsTransport` (a new DPoP-signed reqwest client). Selection happens at process startup based on the presence of `~/.config/teramind/team.toml`. Read tools (`search`, `recall`, `wiki`, `auto_recall`) wrap their transport in a fallback adapter that runs `grep_fallback::run` over the local JSONL shadow log when the transport returns a connection error; `save_skill` does not fall back.

**Tech Stack:** Rust 1.93. Reuses Plan I's `teramind-sync-server::auth` middleware (already attaches `AuthContext`), Plan I's `route_with_deps` pattern (template), and Plan J's `teramind_core::dpop::sign` helpers. No new workspace deps.

---

## Spec coverage

This plan implements spec §3.1 (MCP server: stdio + HTTPS backend), §7.1–§7.6 (RpcTransport, /v1/rpc, four MCP tools across modes, grep fallback, team-mode auto-recall, latency budgets), §10.3 (multi-daemon L3 harness). Coverage matrix at the bottom.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramindd/src/services/rpc_dispatch.rs` | Free `dispatch(deps, req, auth)` fn + `RpcDeps` struct (extracted from `ipc_server`) |
| `crates/teramind-core/src/rpc_transport.rs` | `RpcTransport` async trait |
| `crates/teramind-core/src/grep_fallback_client.rs` | Read-side fallback wrapper |
| `crates/teramind-mcp/src/transport_local.rs` | `LocalIpcTransport` impl |
| `crates/teramind-mcp/src/transport_https.rs` | `HttpsTransport` impl (DPoP-signed) |
| `crates/teramind-sync-server/src/handlers/rpc.rs` | `POST /v1/rpc` axum handler |
| `crates/teramind-sync-server/tests/rpc_endpoint.rs` | Server-side `/v1/rpc` integration test |
| `crates/teramind-mcp/tests/transport_https.rs` | Client-side HTTPS round-trip + grep fallback test |
| `crates/teramindd/tests/two_dev_team_mode.rs` | Two-developer E2E (alice writes, bob reads via MCP→server) |

**Modified files:**

- `crates/teramindd/src/services/ipc_server.rs` — `handle_request` body becomes `dispatch(&self.rpc_deps(), req, None).await`.
- `crates/teramindd/src/services/mod.rs` — register `rpc_dispatch`.
- `crates/teramindd/src/lib.rs` — re-export `RpcDeps`, `dispatch` for the server.
- `crates/teramind-sync-server/src/server.rs` — register `POST /v1/rpc` behind the auth layer.
- `crates/teramind-sync-server/src/state.rs` — `AppState::rpc_deps()` factory parallel to `route_deps()`; builds the read-side state.
- `crates/teramind-core/src/lib.rs` — register `rpc_transport` + `grep_fallback_client`.
- `crates/teramind-mcp/src/server.rs` — select transport at startup; route all four tools through it.
- `crates/teramind-hook/src/auto_recall.rs` — accept a transport instead of a hardcoded `IpcClient`.

---

## Section 0 — Pre-flight

### Task 0.1: Branch from a green main

- [ ] Run:
```bash
git checkout main
cargo build --workspace
git checkout -b feat/teramind-mcp-proxy
```

Expect: workspace build silent; HEAD on new branch. Plan J merged, baseline is 321 tests.

### Task 0.2: Confirm Plan J merge

- [ ] `git log --oneline -5` shows a Plan J merge commit. If not, abort and resolve before proceeding.

---

## Section 1 — Extract `dispatch` + `RpcDeps`

### Task 1.1: Read the existing handler

- [ ] `grep -n 'fn handle_request\|Request::\|Response::' crates/teramindd/src/services/ipc_server.rs | head -40`. Confirm the variants currently dispatched are: `Status`, `Ping`, `Shutdown`, `Search`, `Recall`, `AutoRecall`, `SaveSkill`, `WikiLookup`, `TeamShareSet`. The first three are daemon-control and stay in `DaemonIpcHandler`. The next four (Search, Recall, AutoRecall, SaveSkill, WikiLookup) are the **RPC-shared** set. `TeamShareSet` is local-only (the marker is per-machine).

### Task 1.2: Define `RpcDeps`

**File:** `crates/teramindd/src/services/rpc_dispatch.rs` (new)

```rust
//! Shared RPC dispatch logic used by both the local daemon's IPC server and
//! the central sync server's POST /v1/rpc handler.

use crate::services::search::BlendWeights;
use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::embed::EmbeddingProvider;
use teramind_core::summarize::SummaryProvider;
use teramind_db::pool::DbPool;
use teramind_db::repos::{SearchRepo, WikiRepo};
use teramind_ipc::proto::{Request, Response};

#[derive(Clone)]
pub struct RpcDeps {
    pub pool: DbPool,
    pub search_repo: SearchRepo,
    pub wiki_repo: WikiRepo,
    pub embed_provider: Arc<dyn EmbeddingProvider>,
    pub embed_model: String,
    pub search_weights: BlendWeights,
    pub summary_provider: Arc<dyn SummaryProvider>,
    pub summary_model: String,
    pub jsonl_dir: PathBuf,
}

/// Identity of the caller — `Some` on the server-side `/v1/rpc` after auth,
/// `None` for local daemon IPC (single-user mode).
#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: uuid::Uuid,
    pub device_id: uuid::Uuid,
}

/// The dispatch body for read + skill-save requests.
///
/// `Status`/`Ping`/`Shutdown` (daemon control) and `TeamShareSet` (local file
/// IO) are NOT handled here — they remain in `DaemonIpcHandler::handle_request`.
pub async fn dispatch(deps: &RpcDeps, req: Request, auth: Option<AuthContext>) -> Response {
    match req {
        Request::Search(r) => {
            let out = crate::services::search::do_search_with_fallback(
                &deps.search_repo,
                &deps.jsonl_dir,
                Some(deps.embed_provider.clone()),
                &deps.embed_model,
                deps.search_weights,
                &r,
            ).await;
            Response::SearchResults(teramind_core::types::SearchResults {
                hits: filter_by_auth(out.hits, auth),
                degraded: out.degraded,
                took_ms: out.took_ms,
            })
        }
        Request::Recall(r) => {
            let out = crate::services::search::do_recall(
                &deps.search_repo,
                Some(deps.embed_provider.clone()),
                &deps.embed_model,
                deps.search_weights,
                &r,
            ).await;
            Response::SearchResults(teramind_core::types::SearchResults {
                hits: filter_by_auth(out.hits, auth),
                degraded: out.degraded,
                took_ms: out.took_ms,
            })
        }
        Request::AutoRecall(r) => {
            let md = crate::services::search::do_auto_recall(
                &deps.search_repo,
                Some(deps.embed_provider.clone()),
                &deps.embed_model,
                deps.search_weights,
                &deps.wiki_repo,
                &r,
            ).await;
            Response::AutoRecallDigest { markdown: md.markdown, degraded: md.degraded }
        }
        Request::SaveSkill(r) => {
            let skill_repo = teramind_db::repos::SkillRepo::new(deps.pool.clone());
            match skill_repo.upsert_authored(&r.name, &r.description, &r.body).await {
                Ok(id) => Response::SkillRef(teramind_core::types::SkillRef {
                    id: id.0.to_string(), name: r.name,
                }),
                Err(e) => Response::Error(e.to_string()),
            }
        }
        Request::WikiLookup { session_id, cwd } => {
            match crate::services::search::do_wiki_lookup(
                &deps.wiki_repo, &deps.summary_model,
                session_id.as_deref(), cwd.as_deref(),
            ).await {
                Ok(Some(page)) => Response::WikiPage {
                    session_id: page.session_id.0.to_string(),
                    cwd: page.cwd, model: page.model, content: page.content,
                    generated_at: page.generated_at,
                },
                Ok(None) => Response::WikiNotFound,
                Err(e)   => Response::Error(e.to_string()),
            }
        }
        // Daemon-control + local-only — not handled here.
        Request::Status | Request::Ping | Request::Shutdown |
        Request::TeamShareSet { .. } => Response::Error("unsupported in shared dispatch".into()),
    }
}

/// Currently a no-op (v1.0 defaults to team-wide reads). Future `--mine` /
/// `--user=…` filters wire into this slot.
fn filter_by_auth(hits: Vec<teramind_core::types::Hit>, _auth: Option<AuthContext>)
    -> Vec<teramind_core::types::Hit> { hits }
```

If `do_wiki_lookup` doesn't already exist with that signature, inspect `services::search` and adapt. The existing `Request::WikiLookup` arm in `ipc_server.rs` is the source of truth — copy whatever it does, hoisted into a free fn.

### Task 1.3: Register

In `crates/teramindd/src/services/mod.rs`: `pub mod rpc_dispatch;`.

In `crates/teramindd/src/lib.rs`: `pub use services::rpc_dispatch::{dispatch, AuthContext, RpcDeps};`.

### Task 1.4: Switch DaemonIpcHandler to delegate

In `crates/teramindd/src/services/ipc_server.rs`:

1. Add an inline `fn rpc_deps(&self) -> RpcDeps` that builds a `RpcDeps` from `self`'s existing fields.
2. Replace each of the `Request::Search`, `Recall`, `AutoRecall`, `SaveSkill`, `WikiLookup` match arms with:

```rust
Request::Search(_) | Request::Recall(_) | Request::AutoRecall(_)
| Request::SaveSkill(_) | Request::WikiLookup { .. } => {
    crate::services::rpc_dispatch::dispatch(&self.rpc_deps(), req, None).await
}
```

(Keep `Status`, `Ping`, `Shutdown`, `TeamShareSet` exactly as they are.)

### Task 1.5: Verify

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramindd -- --test-threads=1
```

Expected: build silent, clippy silent, all 119+ teramindd tests still pass — the refactor is behavior-preserving.

### Task 1.6: Commit

```bash
git add crates/teramindd/src/services/rpc_dispatch.rs \
        crates/teramindd/src/services/ipc_server.rs \
        crates/teramindd/src/services/mod.rs \
        crates/teramindd/src/lib.rs
git commit -m "refactor(daemon): extract rpc_dispatch::dispatch + RpcDeps"
```

---

## Section 2 — POST /v1/rpc on the server

### Task 2.1: Build RpcDeps in AppState

**File:** `crates/teramind-sync-server/src/state.rs`

Add an `rpc_deps()` method parallel to the existing `route_deps()`. The server needs:

```rust
use teramindd::services::rpc_dispatch::RpcDeps;
use teramindd::services::search::BlendWeights;
use teramind_db::repos::{SearchRepo, WikiRepo};

impl AppState {
    pub fn rpc_deps(&self) -> RpcDeps {
        RpcDeps {
            pool: self.pool.clone(),
            search_repo: SearchRepo::new(self.pool.clone()),
            wiki_repo: WikiRepo::new(self.pool.clone()),
            embed_provider: self.embed_provider.clone(),
            embed_model: self.embed_model.clone(),
            search_weights: BlendWeights::default(),
            summary_provider: self.summary_provider.clone(),
            summary_model: self.summary_model.clone(),
            jsonl_dir: std::path::PathBuf::new(), // unused server-side
        }
    }
}
```

**This requires `AppState` to hold an `embed_provider` + `summary_provider`.** Plan I's `AppState` did not — those workers don't run on the server *yet*. We need to add them.

For Plan K, the minimum: construct a no-op embedding provider + a no-op summary provider in `AppState::new` (so dispatch can call them, the search will fall back to lexical-only, summaries don't pre-generate — but `WikiLookup` returns cached rows fine). Use the existing `teramindd::services::embed::null::NullEmbeddingProvider` and `teramindd::services::summarize::null::NullSummaryProvider` patterns. If those don't exist, create thin null impls under `teramind-core::embed::Null` / `teramind-core::summarize::Null`.

Add to `crates/teramind-sync-server/src/state.rs`:

```rust
pub struct AppState {
    // …existing fields…
    pub embed_provider: Arc<dyn teramind_core::embed::EmbeddingProvider>,
    pub embed_model: String,
    pub summary_provider: Arc<dyn teramind_core::summarize::SummaryProvider>,
    pub summary_model: String,
}

impl AppState {
    pub fn new(pool: DbPool, cfg: ServerConfig) -> Self {
        // …existing replay-cache + repo construction…
        Self {
            // …existing…
            embed_provider: Arc::new(teramindd::services::embed::null::NullEmbeddingProvider),
            embed_model: "null".into(),
            summary_provider: Arc::new(teramindd::services::summarize::null::NullSummaryProvider),
            summary_model: "null".into(),
        }
    }
}
```

If the null types live at different paths, find them — `grep -rn 'NullEmbedding\|NullSummary' crates/` — and import accordingly.

### Task 2.2: Write the handler

**File:** `crates/teramind-sync-server/src/handlers/rpc.rs`

```rust
//! POST /v1/rpc — auth + DPoP-protected RPC. Dispatches the request enum to
//! the same handler the local daemon uses.

use crate::state::{AppState, AuthContext};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use teramind_ipc::proto::{Request, Response};

pub async fn rpc(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(req): Json<Request>,
) -> impl IntoResponse {
    let auth = teramindd::services::rpc_dispatch::AuthContext {
        user_id: auth.user_id.0,
        device_id: auth.device_id.0,
    };
    let deps = state.rpc_deps();
    let resp: Response = teramindd::services::rpc_dispatch::dispatch(&deps, req, Some(auth)).await;
    (StatusCode::OK, Json(resp))
}
```

Note: the server's `AuthContext` (in `state.rs`) wraps newtype IDs (`UserId(Uuid)`, `DeviceId(Uuid)`); `teramindd::services::rpc_dispatch::AuthContext` wraps raw `uuid::Uuid`. Convert at the boundary.

### Task 2.3: Wire the route + register module

**File:** `crates/teramind-sync-server/src/server.rs`

Add `pub mod rpc;` in `handlers/mod.rs` if not present. In `build_router`, add to the authed block (next to `/v1/ingest`):

```rust
.route("/v1/rpc", post(handlers::rpc::rpc))
```

### Task 2.4: Failing test

**File:** `crates/teramind-sync-server/tests/rpc_endpoint.rs`

Test the server-side endpoint end-to-end: redeem an invite to get a real client, then POST a `Request::Search` to `/v1/rpc` and verify a `Response::SearchResults` comes back (empty results are fine — we just need the dispatch to round-trip).

Use the same `boot_server` + `redeem` + `signed` helpers Plan I's `tests/ingest_endpoint.rs` already has. Copy them locally (no shared test-helpers crate exists). Submit `Request::Search(SearchRequest { query: "noop".into(), limit: 10, json: false, grep: false })` and assert the response deserializes as `Response::SearchResults`.

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramind-sync-server --test rpc_endpoint -- --test-threads=1`. Expected: 1 PASS.

### Task 2.5: Verify + commit

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind-sync-server --test rpc_endpoint -- --test-threads=1
git add crates/teramind-sync-server/src/state.rs \
        crates/teramind-sync-server/src/handlers/rpc.rs \
        crates/teramind-sync-server/src/handlers/mod.rs \
        crates/teramind-sync-server/src/server.rs \
        crates/teramind-sync-server/tests/rpc_endpoint.rs
git commit -m "feat(sync-server): POST /v1/rpc reuses dispatch + AuthContext"
```

---

## Section 3 — RpcTransport trait

### Task 3.1: Define the trait

**File:** `crates/teramind-core/src/rpc_transport.rs`

```rust
//! Pluggable transport for MCP and hook RPC. Two impls live in
//! `teramind-mcp`: `LocalIpcTransport` (UDS/named pipe to the local
//! daemon) and `HttpsTransport` (DPoP-signed reqwest to the sync server).

use async_trait::async_trait;
use teramind_ipc::proto::{Request, Response};

#[async_trait]
pub trait RpcTransport: Send + Sync {
    async fn request(&self, req: Request) -> Result<Response, RpcError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("connection failure: {0}")]
    Connect(String),
    #[error("server returned non-success: {0}")]
    Server(String),
    #[error("deserialize: {0}")]
    Decode(String),
    #[error("other: {0}")]
    Other(String),
}

impl RpcError {
    pub fn is_connect(&self) -> bool { matches!(self, RpcError::Connect(_)) }
}
```

Register: `pub mod rpc_transport;` in `crates/teramind-core/src/lib.rs`.

Add `async-trait = { workspace = true }` to `[dependencies]` in `crates/teramind-core/Cargo.toml` if not present.

### Task 3.2: Commit

```bash
git add crates/teramind-core/Cargo.toml \
        crates/teramind-core/src/rpc_transport.rs \
        crates/teramind-core/src/lib.rs
git commit -m "feat(core): RpcTransport trait"
```

---

## Section 4 — LocalIpcTransport + HttpsTransport

### Task 4.1: LocalIpcTransport

**File:** `crates/teramind-mcp/src/transport_local.rs`

```rust
//! UDS / named-pipe transport — wraps the existing teramind-ipc::IpcClient.

use async_trait::async_trait;
use teramind_core::rpc_transport::{RpcError, RpcTransport};
use teramind_ipc::client::IpcClient;
use teramind_ipc::proto::{Request, Response};

pub struct LocalIpcTransport {
    socket_path: std::path::PathBuf,
}

impl LocalIpcTransport {
    pub fn new(socket_path: std::path::PathBuf) -> Self { Self { socket_path } }
}

#[async_trait]
impl RpcTransport for LocalIpcTransport {
    async fn request(&self, req: Request) -> Result<Response, RpcError> {
        let mut client = IpcClient::connect(&self.socket_path).await
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        client.send(&req).await.map_err(|e| RpcError::Other(e.to_string()))?;
        client.recv().await.map_err(|e| RpcError::Decode(e.to_string()))
    }
}
```

If `IpcClient::connect` / `send` / `recv` have different shapes, adapt — the goal is to wrap the existing client and surface failures via `RpcError`.

### Task 4.2: HttpsTransport

**File:** `crates/teramind-mcp/src/transport_https.rs`

```rust
//! DPoP-signed HTTPS transport. POSTs to {server}/v1/rpc.

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use std::sync::Arc;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::rpc_transport::{RpcError, RpcTransport};
use teramind_core::team::TeamConfig;
use teramind_ipc::proto::{Request, Response};
use time::OffsetDateTime;

pub struct HttpsTransport {
    cfg: Arc<TeamConfig>,
    key: Arc<SigningKey>,
    http: reqwest::Client,
}

impl HttpsTransport {
    pub fn new(cfg: Arc<TeamConfig>, key: Arc<SigningKey>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build().expect("reqwest client");
        Self { cfg, key, http }
    }
}

#[async_trait]
impl RpcTransport for HttpsTransport {
    async fn request(&self, req: Request) -> Result<Response, RpcError> {
        let url = format!("{}/v1/rpc", self.cfg.server_url);
        let body = serde_json::to_vec(&req).map_err(|e| RpcError::Other(e.to_string()))?;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = ProofClaims {
            htm: "POST".into(), htu: url.clone(), iat: now,
            jti: format!("jti-{}", uuid::Uuid::new_v4()),
            ath: token_hash_hex(&self.cfg.device_token),
            bsh: body_hash_hex(&body),
        };
        let proof = sign(&claims, &self.key);
        let resp = self.http.post(&url)
            .header("Authorization", format!("Bearer {}", self.cfg.device_token))
            .header("X-Teramind-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body).send().await
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(RpcError::Server(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default())));
        }
        resp.json::<Response>().await.map_err(|e| RpcError::Decode(e.to_string()))
    }
}
```

Add to `crates/teramind-mcp/Cargo.toml` if missing:
- `async-trait = { workspace = true }`
- `ed25519-dalek = { workspace = true }`
- `reqwest = { workspace = true }`
- `time = { workspace = true }`
- `uuid = { workspace = true }`
- `serde_json = { workspace = true }`
- `teramind-core = { path = "../teramind-core" }`

Register both modules in `crates/teramind-mcp/src/lib.rs` (or wherever the mcp crate's modules are declared — check first).

### Task 4.3: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
git add crates/teramind-mcp/Cargo.toml \
        crates/teramind-mcp/src/transport_local.rs \
        crates/teramind-mcp/src/transport_https.rs \
        crates/teramind-mcp/src/lib.rs
git commit -m "feat(mcp): LocalIpc + Https RpcTransport impls"
```

---

## Section 5 — Grep fallback wrapper

### Task 5.1: Implement

**File:** `crates/teramind-core/src/grep_fallback_client.rs`

```rust
//! Wraps an `RpcTransport` and falls back to grep over local JSONL for read
//! tools (`Search`, `Recall`, `AutoRecall`, `WikiLookup`) when the transport
//! reports a connect failure. Writes (`SaveSkill`) and daemon-control are
//! never falled back.

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use crate::rpc_transport::{RpcError, RpcTransport};
use teramind_ipc::proto::{Request, Response};

pub struct GrepFallback {
    inner: Arc<dyn RpcTransport>,
    jsonl_dir: PathBuf,
}

impl GrepFallback {
    pub fn new(inner: Arc<dyn RpcTransport>, jsonl_dir: PathBuf) -> Self {
        Self { inner, jsonl_dir }
    }
}

#[async_trait]
impl RpcTransport for GrepFallback {
    async fn request(&self, req: Request) -> Result<Response, RpcError> {
        let is_read = matches!(req,
            Request::Search(_) | Request::Recall(_) | Request::AutoRecall(_)
            | Request::WikiLookup { .. });
        match self.inner.request(req.clone()).await {
            Ok(r) => Ok(r),
            Err(e) if !e.is_connect() => Err(e),
            Err(_) if !is_read => Err(RpcError::Connect("server unreachable; write refused".into())),
            Err(_) => fallback(&req, &self.jsonl_dir).await,
        }
    }
}

async fn fallback(req: &Request, jsonl_dir: &std::path::Path) -> Result<Response, RpcError> {
    match req {
        Request::Search(r) => {
            // Reuse the daemon's grep_fallback (lives in teramindd).
            // For client-side fallback we duplicate the minimal grep logic so
            // teramind-core stays daemon-independent. Lines that contain the
            // query are returned as Hits with degraded=true.
            let q = r.query.to_lowercase();
            let mut hits: Vec<teramind_core::types::Hit> = vec![];
            if let Ok(rd) = std::fs::read_dir(jsonl_dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|s| s.to_str()) != Some("jsonl") { continue; }
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        for line in text.lines().take(10_000) {
                            if line.to_lowercase().contains(&q) {
                                // Minimal hit: GrepLine variant.
                                hits.push(teramind_core::types::Hit::GrepLine {
                                    path: p.display().to_string(),
                                    line: line.to_string(),
                                });
                                if hits.len() as u32 >= r.limit { break; }
                            }
                        }
                    }
                    if hits.len() as u32 >= r.limit { break; }
                }
            }
            Ok(Response::SearchResults(teramind_core::types::SearchResults {
                hits, degraded: true, took_ms: 0,
            }))
        }
        Request::Recall(_) | Request::AutoRecall(_) | Request::WikiLookup { .. } => {
            // For these, return empty + degraded.
            Ok(Response::SearchResults(teramind_core::types::SearchResults {
                hits: vec![], degraded: true, took_ms: 0,
            }))
        }
        _ => Err(RpcError::Connect("not falled back".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc_transport::RpcTransport;

    struct AlwaysConnectFail;
    #[async_trait]
    impl RpcTransport for AlwaysConnectFail {
        async fn request(&self, _: Request) -> Result<Response, RpcError> {
            Err(RpcError::Connect("forced".into()))
        }
    }

    #[tokio::test]
    async fn search_falls_back_to_grep_on_connect_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("2026-05-17.jsonl"),
            "{\"client_event_id\":\"00000000-0000-0000-0000-000000000001\",\"ts\":\"2026-05-17T00:00:00Z\",\"event\":{\"type\":\"user_prompt\",\"session_id\":\"00000000-0000-0000-0000-000000000002\",\"turn_ordinal\":0,\"prompt\":\"hello world\"}}\n",
        ).unwrap();
        let g = GrepFallback::new(Arc::new(AlwaysConnectFail), dir.path().to_path_buf());
        let r = g.request(Request::Search(teramind_core::types::SearchRequest {
            query: "hello".into(), limit: 5, json: false, grep: false,
        })).await.unwrap();
        match r {
            Response::SearchResults(s) => {
                assert!(s.degraded);
                assert_eq!(s.hits.len(), 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn save_skill_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let g = GrepFallback::new(Arc::new(AlwaysConnectFail), dir.path().to_path_buf());
        let r = g.request(Request::SaveSkill(teramind_core::types::SaveSkillRequest {
            name: "x".into(), description: "y".into(), body: "z".into(),
            source_session_ids: vec![],
        })).await;
        assert!(matches!(r, Err(RpcError::Connect(_))));
    }
}
```

If `Hit::GrepLine` doesn't exist, use whatever `Hit` variant `grep_fallback::run` in teramindd produces. Inspect: `grep -n 'enum Hit\|GrepLine\|pub enum' crates/teramind-core/src/types/hit.rs`.

Register: `pub mod grep_fallback_client;` in `crates/teramind-core/src/lib.rs`.

### Task 5.2: Verify + commit

```bash
cargo test -p teramind-core grep_fallback_client::
cargo clippy -p teramind-core --all-targets -- -D warnings
git add crates/teramind-core/src/grep_fallback_client.rs crates/teramind-core/src/lib.rs
git commit -m "feat(core): GrepFallback wrapper for read-side RpcTransport"
```

---

## Section 6 — teramind-mcp uses RpcTransport

### Task 6.1: Inspect current MCP transport usage

```bash
grep -n 'IpcClient\|connect_socket' crates/teramind-mcp/src/server.rs | head
```

Each tool currently calls `IpcClient::connect(socket).send(req).recv()` (or similar). Replace those with `self.transport.request(req).await`.

### Task 6.2: Build the transport at startup

In `crates/teramind-mcp/src/main.rs` (or wherever the MCP server is instantiated), select transport based on team mode:

```rust
let team_toml = teramind_core::team::default_config_dir().join("team.toml");
let transport: std::sync::Arc<dyn teramind_core::rpc_transport::RpcTransport> =
    if team_toml.exists() {
        let cfg = teramind_core::team::TeamConfig::load(&team_toml)?;
        let key = teramind_core::team::load_signing_key(
            &teramind_core::team::default_config_dir().join("team-key"))?;
        let raw = std::sync::Arc::new(teramind_mcp::transport_https::HttpsTransport::new(
            std::sync::Arc::new(cfg),
            std::sync::Arc::new(key),
        )) as std::sync::Arc<dyn teramind_core::rpc_transport::RpcTransport>;
        // Wrap in GrepFallback so reads degrade gracefully when the server is unreachable.
        let jsonl = teramindd_paths_raw_dir();
        std::sync::Arc::new(teramind_core::grep_fallback_client::GrepFallback::new(raw, jsonl))
            as std::sync::Arc<dyn teramind_core::rpc_transport::RpcTransport>
    } else {
        let sock = teramindd_paths_ipc_socket();
        std::sync::Arc::new(teramind_mcp::transport_local::LocalIpcTransport::new(sock))
            as std::sync::Arc<dyn teramind_core::rpc_transport::RpcTransport>
    };
```

`teramindd_paths_*` placeholders: pull the actual functions out of `teramindd::paths` (or `teramind::paths`). The hook crate already does this; copy its pattern.

### Task 6.3: Update each tool body

For each of `search`, `recall`, `save_skill`, `wiki`, replace the body that constructed an IPC client with:

```rust
let resp = self.transport.request(Request::Search(req)).await
    .map_err(|e| /* surface as MCP error */ ...)?;
match resp { Response::SearchResults(s) => /* return JSON */, other => /* surface */ }
```

Adapt the existing rmcp-1.7 macro patterns.

### Task 6.4: Verify

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind-mcp -- --test-threads=1
```

Existing MCP tests should keep passing (in local-first mode, `LocalIpcTransport` wraps the same `IpcClient` they used).

### Task 6.5: Commit

```bash
git add crates/teramind-mcp
git commit -m "feat(mcp): route all tools through RpcTransport (local-ipc vs https)"
```

---

## Section 7 — teramind-hook auto-recall uses RpcTransport

### Task 7.1: Refactor

**File:** `crates/teramind-hook/src/auto_recall.rs`

The hook's auto-recall path currently opens an `IpcClient` directly. Change the signature so it accepts an `Arc<dyn RpcTransport>` (constructed in `main.rs`):

```rust
pub async fn run(transport: Arc<dyn RpcTransport>, req: AutoRecallRequest) -> String {
    match transport.request(Request::AutoRecall(req)).await {
        Ok(Response::AutoRecallDigest { markdown, degraded: _ }) => markdown,
        Ok(_) | Err(_) => String::new(),
    }
}
```

In `crates/teramind-hook/src/main.rs`, construct the transport the same way `teramind-mcp` does in §6.2. Wrap in `GrepFallback` for team mode.

### Task 7.2: Verify

```bash
cargo build -p teramind-hook
cargo clippy -p teramind-hook --all-targets -- -D warnings
cargo test -p teramind-hook -- --test-threads=1
```

Existing hook tests must continue to pass.

### Task 7.3: Commit

```bash
git add crates/teramind-hook
git commit -m "feat(hook): auto-recall uses RpcTransport"
```

---

## Section 8 — Two-developer E2E test

### Task 8.1: Integration test

**File:** `crates/teramindd/tests/two_dev_team_mode.rs`

```rust
//! Two developers, one server. Alice captures a session (via the forwarder).
//! Bob (different user) opens the MCP tool surface against the same server
//! and finds Alice's content via `mcp__teramind__search`.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_core::rpc_transport::RpcTransport;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_ipc::proto::{Request, Response};
use teramind_mcp::transport_https::HttpsTransport;
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use teramindd::services::team_sync::{TeamSync, TeamSyncDeps};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alice_captures_bob_searches() -> anyhow::Result<()> {
    // Boot server.
    let pg_dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(pg_dir.path().to_path_buf(), "teramind").await?;
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

    let make_redeemer = |email: &str| async {
        let invites = InviteRepo::new(pool.clone());
        let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes().to_vec();
        let code = InviteCode::generate(&mut OsRng);
        invites.create(&code.hash(), email, None, None,
                       OffsetDateTime::now_utc() + TDur::days(7)).await.unwrap();
        let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
            .json(&json!({
                "invite_code": code.as_str(), "device_name": email,
                "device_public_key_b64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD, &pk),
            })).send().await.unwrap();
        let body: serde_json::Value = r.json().await.unwrap();
        let cfg = TeamConfig {
            server_url: format!("http://{addr}"),
            user_email: email.into(),
            user_id: body["user_id"].as_str().unwrap().into(),
            device_id: body["device_id"].as_str().unwrap().into(),
            device_token: body["device_token"].as_str().unwrap().into(),
            device_name: email.into(),
            redeemed_at: OffsetDateTime::now_utc(),
        };
        (Arc::new(cfg), Arc::new(sk))
    };

    let (alice_cfg, alice_sk) = make_redeemer("alice@acme.dev").await;
    let (_bob_cfg, _bob_sk)   = make_redeemer("bob@acme.dev").await;

    // Alice's daemon writes a session via the forwarder.
    let raw_dir = tempfile::tempdir()?;
    let jsonl = raw_dir.path().join("2026-05-17.jsonl");
    let sid = Uuid::new_v4();
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let envelope = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": started.format(&time::format_description::well_known::Rfc3339)?,
        "event": { "type": "user_prompt", "session_id": sid.to_string(),
                   "turn_ordinal": 0, "prompt": "openvms fork autoconf probe" }
    });
    let session_envelope = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": started.format(&time::format_description::well_known::Rfc3339)?,
        "event": { "type": "session_start", "session_id": sid.to_string(),
                   "agent_kind": "claude_code", "cwd": "/x",
                   "os": "linux", "hostname": "h", "user_login": "u",
                   "git_head": null, "git_branch": null, "agent_session_id": null }
    });
    std::fs::write(&jsonl, format!("{}\n{}\n",
        serde_json::to_string(&session_envelope)?,
        serde_json::to_string(&envelope)?,
    ))?;
    let cache = DecisionCache::new();
    cache.set_initial(SessionId(sid), ShareDecision::Allowed);
    let _forwarder = TeamSync::spawn(TeamSyncDeps {
        team_cfg: alice_cfg.clone(),
        signing_key: alice_sk.clone(),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8, max_attempts: 3,
    });

    // Wait for landing.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns WHERE session_id = $1")
            .bind(sid).fetch_one(pool.pg()).await?;
        if n >= 1 { break; }
    }

    // Bob queries over HTTPS RPC. (Bob's own daemon would do this; here we
    // shortcut by constructing an HttpsTransport directly.)
    let bob_transport = HttpsTransport::new(_bob_cfg, _bob_sk);
    let r = bob_transport.request(Request::Search(teramind_core::types::SearchRequest {
        query: "fork autoconf".into(), limit: 10, json: false, grep: false,
    })).await?;
    match r {
        Response::SearchResults(s) => {
            assert!(!s.hits.is_empty(), "Bob must find Alice's content via team-wide search");
        }
        other => panic!("unexpected response: {other:?}"),
    }

    sup.shutdown().await?;
    Ok(())
}
```

Add to `crates/teramindd/Cargo.toml` `[dev-dependencies]`:
- `teramind-mcp = { path = "../teramind-mcp" }` (if not present)

### Task 8.2: Verify + commit

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramindd --test two_dev_team_mode -- --test-threads=1
git add crates/teramindd/Cargo.toml crates/teramindd/tests/two_dev_team_mode.rs
git commit -m "test(daemon): two-developer team-mode E2E (alice writes, bob reads)"
```

---

## Section 9 — Final check

### Task 9.1: Workspace sweep

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plan J baseline: 321 tests. Plan K adds approximately:
- §2 rpc_endpoint: 1
- §5 grep_fallback_client: 2
- §8 two_dev_team_mode: 1

Expected total: ~325. (Existing tests should keep passing — the §1 refactor is behavior-preserving for local-first mode.)

### Task 9.2: Report

Print HEAD SHA, total commit count from main, total tests, any failures. Do NOT push.

---

## Spec coverage matrix

| Spec section | Plan K addresses | Notes |
|---|---|---|
| §2.1 In-scope — MCP proxy | §1–§7 | — |
| §2.1 In-scope — read-path fallback | §5 | — |
| §3.1 MCP backend split | §6 (transport selection) | — |
| §7.1 RpcTransport trait | §3 | — |
| §7.2 POST /v1/rpc reuses dispatch | §1 (extract), §2 (endpoint) | — |
| §7.3 Same four tools work in either mode | §6 | — |
| §7.4 Grep fallback | §5 | — |
| §7.5 SessionStart auto-recall in team mode | §7 (hook uses HttpsTransport) | — |
| §7.6 Latency | — | Verified by benchmarking after this lands |
| §10.3 L3 multi-daemon harness | §8 | — |

---
