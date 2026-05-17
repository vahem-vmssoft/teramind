# Teramind Team Forwarder (Plan J) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make team mode usable end-to-end on a developer's machine. The local daemon, when team-mode-configured, captures sessions exactly as before, then forwards every captured event over HTTPS to the central `teramind-sync-server` (Plan I). Per-project privacy is gated by an agent-driven prompt that writes a marker file; sessions hold their events locally until consent, then backfill on share=true.

**Architecture:** A new local CLI flow (`teramind init --team --server=… --invite=…`) generates an Ed25519 keypair, redeems the invite against `/v1/auth/redeem`, and writes `~/.config/teramind/{team.toml, team-key}` at 0600. A new `team_sync` service inside the daemon tails the JSONL shadow log with a persisted offset, batches events (32 per batch), DPoP-signs each request, and POSTs to `/v1/ingest`. A `DecisionCache` keyed by `SessionId` holds `Pending | Allowed | DeniedKeepLocal` state per session; ship only happens for `Allowed`. On `SessionStart`, the hook walks cwd → `$HOME` looking for `.teramind/team-share.toml`; if absent, the hook injects an agent-facing notice ("ask the user whether to share this project"). The agent answers by calling `mcp__teramind__team_share_set(scope, share)` which writes the marker, updates the cache, and (on `share=true`) triggers backfill of held events.

**Tech Stack:** Rust 1.93. Reuses Plan I's `proof::sign` (refactored into `teramind-core::dpop` in §1 so both the client and server share signing code; the server keeps `verify` + replay). New deps: none — `reqwest` already in the daemon for cloud providers; `ed25519-dalek` already in the workspace.

---

## Spec coverage

This plan implements §5.2 (device redemption from the client), §5.4–§5.5 (defense properties + doctor surfaces), §6 (capture forwarding end-to-end), and the parts of §2.1 that require client work to be usable. Coverage matrix at the bottom.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramind-core/src/dpop.rs` | Shared DPoP types: `ProofClaims`, `sign`, `body_hash_hex`, `token_hash_hex` |
| `crates/teramind-core/src/team.rs` | `TeamConfig` (deserialize team.toml) + secure-file helpers |
| `crates/teramind/src/commands/init_team.rs` | `teramind init --team --server --invite` body |
| `crates/teramindd/src/services/team_sync.rs` | Tail-JSONL forwarder loop |
| `crates/teramindd/src/services/decision_cache.rs` | `ShareDecision` state machine + cache |
| `crates/teramindd/src/services/team_share.rs` | `.teramind/team-share.toml` walker + writer |
| `crates/teramindd/src/services/sync_offset.rs` | Persisted offset file IO |
| `crates/teramindd/tests/team_forwarder_e2e.rs` | End-to-end forwarder integration test |
| `crates/teramindd/tests/team_share_decision_flow.rs` | Hold-and-backfill privacy test |

**Modified files:**

- `crates/teramind-sync-server/src/proof.rs` — re-export `sign`/`ProofClaims`/`body_hash_hex`/`token_hash_hex` from `teramind_core::dpop` for backward compat; keep `verify` + `replay` here.
- `crates/teramind-core/src/lib.rs` — register `dpop` + `team` modules.
- `crates/teramind/src/cli.rs` — add `--team --server --invite --device-name` flags to the `init` subcommand.
- `crates/teramind/src/commands/init.rs` — branch into `init_team::run` when `--team` is passed.
- `crates/teramind-ipc/src/proto.rs` — add `Request::TeamShareSet { session_id, scope, share }` and `Response::Ok` reused.
- `crates/teramindd/src/services/ipc_server.rs` — handle `TeamShareSet` (write marker, update cache, notify forwarder).
- `crates/teramindd/src/services/mod.rs` — register `team_sync`, `decision_cache`, `team_share`, `sync_offset`.
- `crates/teramindd/src/app.rs` — read team.toml on startup; spawn `team_sync` if present.
- `crates/teramind-hook/src/translate.rs` — on `SessionStart`, inject the share-prompt notice if no marker exists.
- `crates/teramind-mcp/src/server.rs` — register `mcp__teramind__team_share_set` tool.
- `crates/teramind/src/commands/doctor.rs` — replace the §19 placeholder line with full team-mode health (server URL, user/device, last-seen, key permissions, forwarder backlog/throughput, hold count).
- `crates/teramind/src/commands/mod.rs` — register `init_team`.

---

## Section 0 — Pre-flight

### Task 0.1: Cut a branch from a green main

- [ ] **Step 1**

Run:
```bash
git fetch origin || true
git checkout main
cargo build --workspace
git checkout -b feat/teramind-team-forwarder
```

Expected: build silent; HEAD now on `feat/teramind-team-forwarder`. Plan I just merged; the new `teramind-sync-server` crate must compile in this baseline.

### Task 0.2: Sanity-test the server boots

- [ ] **Step 1: Print version**

Run: `./target/debug/teramind-sync-server version`

Expected: `teramind-sync-server 0.1.0`. If the binary doesn't exist, run `cargo build -p teramind-sync-server` first.

---

## Section 1 — Extract DPoP sign helpers into teramind-core

The client (Plan J) needs to *sign* requests; only the server needs to *verify*. Move the signing primitives + claim struct to `teramind-core` so both crates share them, and keep `verify` + replay-cache server-side.

### Task 1.1: Create the shared dpop module

**Files:**
- Create: `crates/teramind-core/src/dpop.rs`
- Modify: `crates/teramind-core/src/lib.rs`

- [ ] **Step 1: Write `dpop.rs`**

```rust
//! Shared DPoP types (RFC 9449 with `ath` + `bsh` additions).
//! Sign + hash helpers are here so both the central server and remote
//! daemons can use them. The server's verify + replay cache stay in
//! `teramind-sync-server::proof`.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofClaims {
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    pub jti: String,
    pub ath: String,
    pub bsh: String,
}

pub fn body_hash_hex(body: &[u8]) -> String {
    let mut h = Sha256::new(); h.update(body); hex::encode(h.finalize())
}

pub fn token_hash_hex(token: &str) -> String {
    let mut h = Sha256::new(); h.update(token.as_bytes()); hex::encode(h.finalize())
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn sign(claims: &ProofClaims, signing_key: &SigningKey) -> String {
    let header = br#"{"alg":"EdDSA","typ":"dpop+jwt"}"#;
    let claims_json = serde_json::to_vec(claims).expect("claims serialize");
    let signing_input = format!("{}.{}", b64url(header), b64url(&claims_json));
    let sig: Signature = signing_key.sign(signing_input.as_bytes());
    format!("{signing_input}.{}", b64url(&sig.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::{rngs::OsRng, RngCore};

    #[test]
    fn sign_produces_three_segments() {
        let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let claims = ProofClaims {
            htm: "POST".into(), htu: "https://srv/v1/ingest".into(),
            iat: 1_700_000_000, jti: "test".into(),
            ath: token_hash_hex("tmd_v1_X"), bsh: body_hash_hex(b""),
        };
        let header = sign(&claims, &sk);
        assert_eq!(header.split('.').count(), 3);
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/teramind-core/src/lib.rs`, add `pub mod dpop;` (alphabetically).

- [ ] **Step 3: Add `base64` + `ed25519-dalek` to teramind-core deps**

Inspect `crates/teramind-core/Cargo.toml`. Add under `[dependencies]`:

```toml
base64        = { workspace = true }
ed25519-dalek = { workspace = true }
hex           = { workspace = true }
```

(`sha2` is already there since Plan A.)

- [ ] **Step 4: Build + test**

Run: `cargo test -p teramind-core dpop::`

Expected: 1 PASS.

### Task 1.2: Switch the server crate to re-export from teramind-core

**Files:**
- Modify: `crates/teramind-sync-server/src/proof.rs`

- [ ] **Step 1: Remove duplicates, re-export from core**

At the top of `proof.rs`, replace the existing `ProofClaims`, `body_hash_hex`, `token_hash_hex`, `sign`, the `b64url_encode` helper, and the imports that supported them, with:

```rust
pub use teramind_core::dpop::{ProofClaims, body_hash_hex, token_hash_hex, sign};
```

Keep `verify`, `ProofError`, `b64url_decode`, and the `replay` submodule unchanged.

- [ ] **Step 2: Verify tests still pass**

Run: `cargo test -p teramind-sync-server proof::`

Expected: 8 + 3 PASS (same as Plan I).

### Task 1.3: Commit

```bash
git add crates/teramind-core/Cargo.toml \
        crates/teramind-core/src/dpop.rs \
        crates/teramind-core/src/lib.rs \
        crates/teramind-sync-server/src/proof.rs
git commit -m "refactor(core): extract DPoP sign helpers into teramind-core::dpop"
```

---

## Section 2 — TeamConfig types + secure-file IO

The client needs to read `team.toml` (server URL, user/device IDs, bearer token, device name) and `team-key` (Ed25519 private key, raw 32-byte file). Both at mode 0600.

### Task 2.1: Failing test

**Files:**
- Create: `crates/teramind-core/src/team.rs` (with tests inline)
- Modify: `crates/teramind-core/src/lib.rs` — register module

- [ ] **Step 1: Write the module + tests**

```rust
//! Team-mode config: ~/.config/teramind/team.toml + team-key (Ed25519 32 bytes).

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub server_url: String,
    pub user_email: String,
    pub user_id: String,
    pub device_id: String,
    pub device_token: String,
    pub device_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub redeemed_at: time::OffsetDateTime,
}

impl TeamConfig {
    pub fn load(path: &Path) -> Result<Self> {
        ensure_secure_perms(path).context("team.toml perms")?;
        let raw = std::fs::read_to_string(path).context("read team.toml")?;
        let cfg: TeamConfig = toml::from_str(&raw).context("parse team.toml")?;
        Ok(cfg)
    }

    /// Writes team.toml with mode 0600. Overwrites if present.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let raw = toml::to_string(self).context("serialize team.toml")?;
        write_secure(path, raw.as_bytes())
    }
}

/// Read the 32-byte raw Ed25519 private key from `team-key`. Enforces 0600.
pub fn load_signing_key(path: &Path) -> Result<SigningKey> {
    ensure_secure_perms(path).context("team-key perms")?;
    let bytes = std::fs::read(path).context("read team-key")?;
    if bytes.len() != 32 {
        return Err(anyhow!("team-key must be exactly 32 bytes (got {})", bytes.len()));
    }
    let mut arr = [0u8; 32]; arr.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&arr))
}

/// Write a 32-byte Ed25519 private key to disk with mode 0600.
pub fn save_signing_key(path: &Path, key: &SigningKey) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create config dir")?;
    }
    write_secure(path, &key.to_bytes())
}

/// Default config directory: $XDG_CONFIG_HOME/teramind or ~/.config/teramind.
pub fn default_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("teramind")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("teramind")
    } else {
        PathBuf::from(".").join(".teramind")
    }
}

fn write_secure(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true).create(true).truncate(true).mode(0o600)
            .open(path).with_context(|| format!("open {}", path.display()))?;
        f.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true).create(true).truncate(true)
            .open(path).with_context(|| format!("open {}", path.display()))?;
        f.write_all(bytes)?;
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_secure_perms(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let md = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let mode = md.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(anyhow!("{} has insecure perms {:#o}; chmod 0600 to fix", path.display(), mode));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_secure_perms(_path: &Path) -> Result<()> { Ok(()) }

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::{rngs::OsRng, RngCore};

    fn random_key() -> SigningKey {
        let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    #[test]
    fn team_config_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.toml");
        let cfg = TeamConfig {
            server_url: "https://srv".into(),
            user_email: "alice@acme.dev".into(),
            user_id: uuid::Uuid::new_v4().to_string(),
            device_id: uuid::Uuid::new_v4().to_string(),
            device_token: "tmd_v1_XYZ".into(),
            device_name: "alice-mac".into(),
            redeemed_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        };
        cfg.save(&path).unwrap();
        let loaded = TeamConfig::load(&path).unwrap();
        assert_eq!(loaded.device_token, "tmd_v1_XYZ");
        assert_eq!(loaded.server_url, "https://srv");
    }

    #[test]
    fn signing_key_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team-key");
        let original = random_key();
        save_signing_key(&path, &original).unwrap();
        let loaded = load_signing_key(&path).unwrap();
        assert_eq!(original.to_bytes(), loaded.to_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_insecure_perms_on_load() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.toml");
        std::fs::write(&path, "bogus").unwrap();
        let mut p = std::fs::metadata(&path).unwrap().permissions();
        p.set_mode(0o644);
        std::fs::set_permissions(&path, p).unwrap();
        assert!(TeamConfig::load(&path).is_err(), "0644 must be rejected");
    }
}
```

- [ ] **Step 2: Register module**

In `crates/teramind-core/src/lib.rs`, add `pub mod team;` (alphabetically).

- [ ] **Step 3: Add `toml` dep to teramind-core**

If not already present in `crates/teramind-core/Cargo.toml`:

```toml
toml = { workspace = true }
```

- [ ] **Step 4: Verify**

Run: `cargo test -p teramind-core team::`

Expected: 3 PASS on Unix (2 on non-Unix).

### Task 2.2: Commit

```bash
git add crates/teramind-core/Cargo.toml \
        crates/teramind-core/src/team.rs \
        crates/teramind-core/src/lib.rs
git commit -m "feat(core): TeamConfig + Ed25519 key file IO (0600 enforced)"
```

---

## Section 3 — `teramind init --team` subcommand

### Task 3.1: CLI surface

**Files:**
- Modify: `crates/teramind/src/cli.rs`

- [ ] **Step 1: Add the flags to `Init`**

Locate the `Init` variant in the `Cli` / `Cmd` enum. Extend with:

```rust
Init {
    /// Opt into team mode: redeem an invite + generate a device key.
    #[arg(long)]
    team: bool,
    /// Sync server URL (required with --team).
    #[arg(long, requires = "team")]
    server: Option<String>,
    /// Invite code from the team admin (required with --team).
    #[arg(long, requires = "team")]
    invite: Option<String>,
    /// Optional device name (defaults to hostname).
    #[arg(long, requires = "team")]
    device_name: Option<String>,
    // …leave existing fields…
},
```

If the existing `Init` variant has other args (eg. `force`), keep them. Add only the four new ones.

### Task 3.2: Wire dispatch

**Files:**
- Modify: `crates/teramind/src/commands/init.rs`
- Modify: `crates/teramind/src/commands/mod.rs`

- [ ] **Step 1: Register `init_team` module**

In `crates/teramind/src/commands/mod.rs`, add `pub mod init_team;`.

- [ ] **Step 2: Branch in init.rs**

At the top of `init`'s body, check the `team` flag:

```rust
if team {
    let server = server.ok_or_else(|| anyhow::anyhow!("--server required with --team"))?;
    let invite = invite.ok_or_else(|| anyhow::anyhow!("--invite required with --team"))?;
    return crate::commands::init_team::run(server, invite, device_name).await;
}
// …existing local-first init path unchanged…
```

### Task 3.3: Implement init_team

**Files:**
- Create: `crates/teramind/src/commands/init_team.rs`

- [ ] **Step 1: Write the subcommand body**

```rust
//! `teramind init --team --server=… --invite=…` body.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use teramind_core::team::{default_config_dir, save_signing_key, TeamConfig};

#[derive(Deserialize)]
struct RedeemResponse {
    user_id: String,
    device_id: String,
    device_token: String,
    device_name: String,
}

pub async fn run(server: String, invite: String, device_name: Option<String>) -> Result<()> {
    let cfg_dir = default_config_dir();
    let team_toml = cfg_dir.join("team.toml");
    if team_toml.exists() {
        return Err(anyhow!(
            "team mode already configured at {}; remove it first to re-init",
            team_toml.display()
        ));
    }

    let server = server.trim_end_matches('/').to_string();
    let device_name = device_name
        .or_else(|| hostname::get().ok().and_then(|s| s.into_string().ok()))
        .unwrap_or_else(|| "unknown-device".into());

    // Generate Ed25519 keypair.
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes();
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode(pk);

    // POST /v1/auth/redeem.
    let body = serde_json::json!({
        "invite_code": invite,
        "device_name": device_name,
        "device_public_key_b64": pk_b64,
    });
    let url = format!("{server}/v1/auth/redeem");
    let resp = reqwest::Client::new().post(&url).json(&body).send().await
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("redeem failed: HTTP {} — {}", status, text));
    }
    let r: RedeemResponse = serde_json::from_str(&text)
        .with_context(|| format!("parse redeem response: {text}"))?;

    let cfg = TeamConfig {
        server_url: server.clone(),
        user_email: "(set by server)".into(), // user_email not in response in v1.0; backfilled at next /v1/health auth round
        user_id: r.user_id,
        device_id: r.device_id,
        device_token: r.device_token,
        device_name: r.device_name,
        redeemed_at: time::OffsetDateTime::now_utc(),
    };
    cfg.save(&team_toml)?;
    save_signing_key(&cfg_dir.join("team-key"), &sk)?;

    println!("team mode configured:");
    println!("  server:  {}", cfg.server_url);
    println!("  device:  {} ({})", cfg.device_name, cfg.device_id);
    println!("  user_id: {}", cfg.user_id);
    println!("  config:  {}", team_toml.display());
    println!("  key:     {} (mode 0600)", cfg_dir.join("team-key").display());
    println!();
    println!("Start the daemon with `teramind start` to begin shipping captures.");
    Ok(())
}
```

### Task 3.4: Add `hostname` dep

In `crates/teramind/Cargo.toml`, under `[dependencies]`:

```toml
hostname = "0.4"
```

And to the workspace `Cargo.toml`:

```toml
hostname = "0.4"
```

(Or use it directly if already present.)

### Task 3.5: Verify

- [ ] `cargo build -p teramind`
- [ ] `cargo clippy -p teramind --all-targets -- -D warnings`

Both silent.

### Task 3.6: Integration test

**Files:**
- Create: `crates/teramind/tests/init_team.rs`

- [ ] **Step 1: Test the redemption flow**

```rust
//! Integration test: real server + real client init flow.

use std::net::SocketAddr;
use teramind_core::team::TeamConfig;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use time::{Duration as TDur, OffsetDateTime};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn init_team_redeems_and_writes_config() -> anyhow::Result<()> {
    let pg_dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(pg_dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    // Spin up sync server.
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(), database_url: "ignored".into(),
        tls: None, auth: AuthConfig::default(), ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    // Issue an invite.
    let invites = teramind_db::repos::InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x55u8; 16]);
    invites.create(&code.hash(), "alice@acme.dev", None, None,
                   OffsetDateTime::now_utc() + TDur::days(7)).await?;

    // Point the client at a sandboxed XDG_CONFIG_HOME.
    let cfg_dir = tempfile::tempdir()?;
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());

    teramind::commands::init_team::run(
        format!("http://{addr}"),
        code.as_str().to_string(),
        Some("test-device".into()),
    ).await?;

    let team_toml = cfg_dir.path().join("teramind").join("team.toml");
    let cfg = TeamConfig::load(&team_toml)?;
    assert_eq!(cfg.device_name, "test-device");
    assert!(cfg.device_token.starts_with("tmd_v1_"));

    let key_path = cfg_dir.path().join("teramind").join("team-key");
    let key = teramind_core::team::load_signing_key(&key_path)?;
    assert_eq!(key.to_bytes().len(), 32);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramind --test init_team -- --test-threads=1
```

Expected: PASS.

### Task 3.7: Commit

```bash
git add crates/teramind/Cargo.toml \
        crates/teramind/src/cli.rs \
        crates/teramind/src/commands/init.rs \
        crates/teramind/src/commands/init_team.rs \
        crates/teramind/src/commands/mod.rs \
        crates/teramind/tests/init_team.rs \
        Cargo.toml
git commit -m "feat(cli): teramind init --team --server --invite"
```

---

## Section 4 — Marker file walker + writer

`.teramind/team-share.toml` lives at any ancestor of the session's cwd (typically at the project root). Format:

```toml
share   = true | false
set_by  = "alice@acme.dev"
set_at  = "2026-05-17T16:12:00Z"
```

### Task 4.1: Failing test

**Files:**
- Create: `crates/teramindd/src/services/team_share.rs`

- [ ] **Step 1: Write module + tests**

```rust
//! Per-project team-share marker: `.teramind/team-share.toml`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareMarker {
    pub share: bool,
    pub set_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub set_at: time::OffsetDateTime,
}

/// Walk from `cwd` upward to `$HOME` looking for `.teramind/team-share.toml`.
/// Returns the first hit, or None.
pub fn find_marker(cwd: &Path, home: &Path) -> Option<(PathBuf, ShareMarker)> {
    let mut dir = cwd.canonicalize().ok()?;
    loop {
        let candidate = dir.join(".teramind").join("team-share.toml");
        if candidate.exists() {
            if let Ok(raw) = std::fs::read_to_string(&candidate) {
                if let Ok(m) = toml::from_str::<ShareMarker>(&raw) {
                    return Some((candidate, m));
                }
            }
        }
        if dir == home { break; }
        let Some(parent) = dir.parent() else { break; };
        if parent == dir { break; }
        dir = parent.to_path_buf();
    }
    None
}

/// Write the marker at `<cwd>/.teramind/team-share.toml`. Creates the dir.
pub fn write_marker_at_cwd(cwd: &Path, marker: &ShareMarker) -> Result<PathBuf> {
    let dir = cwd.join(".teramind");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("team-share.toml");
    let raw = toml::to_string(marker)?;
    std::fs::write(&path, raw)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_marker_in_self() {
        let dir = tempfile::tempdir().unwrap();
        let marker = ShareMarker {
            share: true, set_by: "alice".into(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(dir.path(), &marker).unwrap();
        let (path, m) = find_marker(dir.path(), &PathBuf::from("/"))
            .expect("marker must be findable from self");
        assert!(path.ends_with("team-share.toml"));
        assert!(m.share);
    }

    #[test]
    fn find_marker_walks_up() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("a/b/c");
        std::fs::create_dir_all(&child).unwrap();
        let marker = ShareMarker {
            share: false, set_by: "alice".into(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(root.path(), &marker).unwrap();
        let (_, m) = find_marker(&child, &PathBuf::from("/"))
            .expect("marker must be findable from a descendant");
        assert!(!m.share);
    }

    #[test]
    fn no_marker_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_marker(dir.path(), &PathBuf::from("/"));
        assert!(result.is_none());
    }

    #[test]
    fn walk_stops_at_home() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        // Marker is *above* HOME — must not be found.
        let marker = ShareMarker {
            share: true, set_by: "x".into(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(root.path(), &marker).unwrap();
        assert!(find_marker(&proj, &home).is_none());
    }
}
```

- [ ] **Step 2: Register module**

In `crates/teramindd/src/services/mod.rs`, add `pub mod team_share;`.

- [ ] **Step 3: Run**

```bash
cargo test -p teramindd services::team_share::
```

Expected: 4 PASS.

### Task 4.2: Commit

```bash
git add crates/teramindd/src/services/team_share.rs \
        crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): team-share marker file walker"
```

---

## Section 5 — IPC: Request::TeamShareSet

### Task 5.1: Add the request variant

**Files:**
- Modify: `crates/teramind-ipc/src/proto.rs`

- [ ] **Step 1: Extend the Request enum**

In `Request`, add:

```rust
    TeamShareSet {
        session_id: Option<String>,
        cwd: String,
        scope: String,  // "project" in v1.0; "session" or "user" reserved
        share: bool,
    },
```

(`Response::Ok` is reused on success; `Response::Error(String)` on failure.)

### Task 5.2: Stub the handler

**Files:**
- Modify: `crates/teramindd/src/services/ipc_server.rs`

- [ ] **Step 1: Add the match arm**

In the `match req` block, add a temporary stub returning `Response::Ok`:

```rust
Request::TeamShareSet { .. } => {
    // Full implementation in §6.
    Response::Ok
}
```

### Task 5.3: Verify the workspace still builds

Run: `cargo build --workspace`

Expected: silent.

### Task 5.4: Commit

```bash
git add crates/teramind-ipc/src/proto.rs \
        crates/teramindd/src/services/ipc_server.rs
git commit -m "feat(ipc): Request::TeamShareSet variant + stub handler"
```

---

## Section 6 — DecisionCache

### Task 6.1: Failing test

**Files:**
- Create: `crates/teramindd/src/services/decision_cache.rs`

- [ ] **Step 1: Write the module + tests**

```rust
//! Per-session ShareDecision state machine.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use teramind_core::ids::SessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareDecision {
    Pending,
    Allowed,
    DeniedKeepLocal,
}

pub struct DecisionCache {
    inner: Mutex<HashMap<SessionId, ShareDecision>>,
}

impl DecisionCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(HashMap::new()) })
    }

    pub fn get(&self, sid: SessionId) -> Option<ShareDecision> {
        self.inner.lock().get(&sid).copied()
    }

    /// Insert if absent; do not overwrite a non-pending state.
    pub fn set_initial(&self, sid: SessionId, d: ShareDecision) {
        let mut m = self.inner.lock();
        m.entry(sid).or_insert(d);
    }

    /// Forcefully update (used when the agent answers).
    /// Returns the previous state.
    pub fn set(&self, sid: SessionId, d: ShareDecision) -> Option<ShareDecision> {
        self.inner.lock().insert(sid, d)
    }

    pub fn evict(&self, sid: SessionId) {
        self.inner.lock().remove(&sid);
    }

    pub fn pending_count(&self) -> usize {
        self.inner.lock().values()
            .filter(|d| **d == ShareDecision::Pending).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sid() -> SessionId { SessionId(Uuid::new_v4()) }

    #[test]
    fn initial_set_does_not_overwrite() {
        let c = DecisionCache::new();
        let s = sid();
        c.set_initial(s, ShareDecision::Allowed);
        c.set_initial(s, ShareDecision::DeniedKeepLocal);
        assert_eq!(c.get(s), Some(ShareDecision::Allowed));
    }

    #[test]
    fn set_overwrites_returns_prev() {
        let c = DecisionCache::new();
        let s = sid();
        c.set_initial(s, ShareDecision::Pending);
        let prev = c.set(s, ShareDecision::Allowed);
        assert_eq!(prev, Some(ShareDecision::Pending));
        assert_eq!(c.get(s), Some(ShareDecision::Allowed));
    }

    #[test]
    fn pending_count() {
        let c = DecisionCache::new();
        c.set_initial(sid(), ShareDecision::Pending);
        c.set_initial(sid(), ShareDecision::Pending);
        c.set_initial(sid(), ShareDecision::Allowed);
        assert_eq!(c.pending_count(), 2);
    }
}
```

- [ ] **Step 2: Register module**

In `crates/teramindd/src/services/mod.rs`, add `pub mod decision_cache;`.

- [ ] **Step 3: Run**

```bash
cargo test -p teramindd services::decision_cache::
```

Expected: 3 PASS.

### Task 6.2: Implement TeamShareSet handler properly

**Files:**
- Modify: `crates/teramindd/src/services/ipc_server.rs`
- Modify: `crates/teramindd/src/app.rs` (next section will spawn the cache + forwarder)

For now, replace the `Request::TeamShareSet` stub with the real implementation, but route through an `Option<Arc<DecisionCache>>` field on `DaemonIpcHandler` (None when team mode isn't configured):

- [ ] **Step 1: Add field to DaemonIpcHandler**

Add to the struct:

```rust
pub decision_cache: Option<std::sync::Arc<crate::services::decision_cache::DecisionCache>>,
pub team_share_writer: Option<std::sync::Arc<TeamShareSetter>>,
```

`TeamShareSetter` is a tiny helper trait declared in the same file so tests can mock it:

```rust
#[async_trait::async_trait]
pub trait TeamShareSetter: Send + Sync {
    async fn write_and_signal(&self, cwd: &std::path::Path,
                              session_id: Option<teramind_core::ids::SessionId>,
                              share: bool, set_by: &str) -> anyhow::Result<()>;
}
```

(Default impl provided by the team_sync wiring in §8.)

- [ ] **Step 2: Replace the match arm**

```rust
Request::TeamShareSet { session_id, cwd, scope: _, share } => {
    let Some(writer) = self.team_share_writer.as_ref() else {
        return Response::Error("team mode not configured".into());
    };
    let sid = session_id.as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(teramind_core::ids::SessionId);
    let cwd_path = std::path::PathBuf::from(&cwd);
    match writer.write_and_signal(&cwd_path, sid, share, "user").await {
        Ok(()) => Response::Ok,
        Err(e) => Response::Error(e.to_string()),
    }
}
```

- [ ] **Step 3: Update tests that construct `DaemonIpcHandler`**

Find every constructor of `DaemonIpcHandler` (likely in `app.rs` and a few tests). Add `decision_cache: None` and `team_share_writer: None` to keep them compiling. Use `grep -rn "DaemonIpcHandler {" crates/`.

- [ ] **Step 4: Verify**

Run: `cargo test -p teramindd --lib`

Expected: existing tests still pass.

### Task 6.3: Commit

```bash
git add crates/teramindd/src/services/decision_cache.rs \
        crates/teramindd/src/services/mod.rs \
        crates/teramindd/src/services/ipc_server.rs
git add -u  # to pick up the constructor updates
git commit -m "feat(daemon): DecisionCache + TeamShareSet IPC plumbing"
```

---

## Section 7 — Sync offset file

A tiny module that persists the JSONL byte offset the forwarder has shipped through, so a daemon restart resumes exactly where it left off.

### Task 7.1: Implementation + tests

**Files:**
- Create: `crates/teramindd/src/services/sync_offset.rs`

- [ ] **Step 1: Write the module**

```rust
//! Persisted forwarder offset.
//!
//! Stored at `<raw_dir>/.sync-offset.json`. The forwarder writes the highest
//! shipped (file, byte-offset) pair after each successful POST /v1/ingest.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncOffset {
    /// JSONL filename relative to raw_dir, e.g. "2026-05-17.jsonl".
    pub file: Option<String>,
    /// Byte offset within that file (next byte to read).
    pub byte_offset: u64,
}

impl SyncOffset {
    pub fn path(raw_dir: &Path) -> PathBuf {
        raw_dir.join(".sync-offset.json")
    }

    pub fn load(raw_dir: &Path) -> Result<Self> {
        let p = Self::path(raw_dir);
        if !p.exists() { return Ok(SyncOffset::default()); }
        let s = std::fs::read_to_string(&p)?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn save(&self, raw_dir: &Path) -> Result<()> {
        let p = Self::path(raw_dir);
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_empty() {
        let dir = tempfile::tempdir().unwrap();
        let off = SyncOffset::load(dir.path()).unwrap();
        assert!(off.file.is_none() && off.byte_offset == 0);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let off = SyncOffset { file: Some("2026-05-17.jsonl".into()), byte_offset: 4096 };
        off.save(dir.path()).unwrap();
        let loaded = SyncOffset::load(dir.path()).unwrap();
        assert_eq!(loaded.file.as_deref(), Some("2026-05-17.jsonl"));
        assert_eq!(loaded.byte_offset, 4096);
    }

    #[test]
    fn save_is_atomic_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let off = SyncOffset { file: Some("x".into()), byte_offset: 100 };
        off.save(dir.path()).unwrap();
        // The tmp file must not linger.
        assert!(!dir.path().join(".sync-offset.json.tmp").exists());
        assert!(dir.path().join(".sync-offset.json").exists());
    }
}
```

- [ ] **Step 2: Register**

In `crates/teramindd/src/services/mod.rs`, add `pub mod sync_offset;`.

- [ ] **Step 3: Verify**

Run: `cargo test -p teramindd services::sync_offset::`

Expected: 3 PASS.

### Task 7.2: Commit

```bash
git add crates/teramindd/src/services/sync_offset.rs \
        crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): sync offset persistence"
```

---

## Section 8 — `team_sync` forwarder

The forwarder is the heart of Plan J. It:
1. Tails the JSONL shadow log (created by Plan A's `JsonlWriter`).
2. Reads from the persisted offset.
3. Filters each event by the `DecisionCache`: `Allowed` ships, `Pending` holds (skipped this round), `DeniedKeepLocal` drops from ship-queue.
4. Batches up to 32 events.
5. DPoP-signs and POSTs to `/v1/ingest` every 1 s or when full.
6. On 200, advances the offset past the batch.
7. On transient error (network, 5xx, 429): exponential backoff 1 s → 60 s; does NOT advance the offset.
8. On permanent error (400, 401, 403): logs loudly; advances the offset past the rejected events so the queue doesn't stall forever.

### Task 8.1: Failing test (E2E happy path)

**Files:**
- Create: `crates/teramindd/tests/team_forwarder_e2e.rs`

- [ ] **Step 1: Test that with `Allowed` decision, captures land in server PG**

```rust
//! End-to-end forwarder integration test:
//! - Spin up the central sync server against an embedded PG.
//! - Redeem an invite to get a real team.toml + team-key.
//! - Start the local forwarder pointing at that server.
//! - Append a SessionStart + AssistantTurn to a JSONL file.
//! - Mark the session Allowed in the DecisionCache.
//! - Assert: the events appear in the server's PG with (user_id, device_id) set.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use teramindd::services::team_sync::{TeamSync, TeamSyncDeps};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

async fn boot_server() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, DbPool)> {
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

async fn redeem(addr: SocketAddr, pool: &DbPool, email: &str)
    -> anyhow::Result<(TeamConfig, SigningKey)>
{
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut OsRng);
    invites.create(&code.hash(), email, None, None,
                   OffsetDateTime::now_utc() + TDur::days(7)).await?;
    let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(),
            "device_name": "test-dev",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        })).send().await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let cfg = TeamConfig {
        server_url: format!("http://{addr}"),
        user_email: email.into(),
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        device_token: body["device_token"].as_str().unwrap().into(),
        device_name: "test-dev".into(),
        redeemed_at: OffsetDateTime::now_utc(),
    };
    Ok((cfg, sk))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarder_ships_allowed_sessions_to_server() -> anyhow::Result<()> {
    let (_pg_dir, sup, addr, pool) = boot_server().await?;
    let (team_cfg, sk) = redeem(addr, &pool, "alice@acme.dev").await?;

    let raw_dir = tempfile::tempdir()?;
    let jsonl = raw_dir.path().join("2026-05-17.jsonl");
    let sid = Uuid::new_v4();
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let envelope = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": started.format(&time::format_description::well_known::Rfc3339)?,
        "event": { "type": "session_start",
                   "session_id": sid.to_string(),
                   "agent_kind": "claude_code", "cwd": "/repo",
                   "os": "linux", "hostname": "h", "user_login": "u",
                   "git_head": null, "git_branch": null, "agent_session_id": null }
    });
    std::fs::write(&jsonl, format!("{}\n", serde_json::to_string(&envelope)?))?;

    let cache = DecisionCache::new();
    cache.set_initial(SessionId(sid), ShareDecision::Allowed);

    let _forwarder = TeamSync::spawn(TeamSyncDeps {
        team_cfg: Arc::new(team_cfg),
        signing_key: Arc::new(sk),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8,
        max_attempts: 3,
    });

    // Wait for the event to land server-side.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
            .bind(sid).fetch_one(pool.pg()).await?;
        if n == 1 { break; }
    }
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sessions WHERE id = $1 AND user_id IS NOT NULL"
    ).bind(sid).fetch_one(pool.pg()).await?;
    assert_eq!(n, 1, "session must arrive at server with user_id annotation");

    sup.shutdown().await?;
    Ok(())
}
```

Run: `export GITHUB_TOKEN=$(gh auth token) && cargo test -p teramindd --test team_forwarder_e2e -- --test-threads=1`

Expected: FAIL — `TeamSync` not yet defined.

### Task 8.2: Implement the forwarder

**Files:**
- Create: `crates/teramindd/src/services/team_sync.rs`

- [ ] **Step 1: Write the module**

```rust
//! Tail-JSONL forwarder. Ships captured events from local JSONL to the
//! central sync server via POST /v1/ingest with DPoP-signed requests.

use crate::services::decision_cache::{DecisionCache, ShareDecision};
use crate::services::sync_offset::SyncOffset;
use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use tokio::io::AsyncBufReadExt;
use tracing::{info, warn};

pub struct TeamSyncDeps {
    pub team_cfg: Arc<TeamConfig>,
    pub signing_key: Arc<SigningKey>,
    pub raw_dir: PathBuf,
    pub cache: Arc<DecisionCache>,
    pub poll_interval: Duration,
    pub batch_size: usize,
    pub max_attempts: u32,
}

pub struct TeamSync {
    _handle: tokio::task::JoinHandle<()>,
}

impl TeamSync {
    pub fn spawn(deps: TeamSyncDeps) -> Self {
        let handle = tokio::spawn(async move { run_loop(deps).await; });
        Self { _handle: handle }
    }
}

async fn run_loop(deps: TeamSyncDeps) {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build().expect("reqwest client");

    loop {
        match tick(&deps, &client).await {
            Ok(true)  => { /* shipped something; loop tight */ }
            Ok(false) => { tokio::time::sleep(deps.poll_interval).await; }
            Err(e)    => {
                warn!(error = %e, "team_sync tick error");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn tick(deps: &TeamSyncDeps, client: &reqwest::Client) -> Result<bool> {
    let mut offset = SyncOffset::load(&deps.raw_dir)?;
    let (path, start_byte) = select_jsonl_file(deps, &offset)?;
    let Some(path) = path else { return Ok(false); };

    let f = tokio::fs::File::open(&path).await
        .with_context(|| format!("open {}", path.display()))?;
    let mut seekable = tokio::io::BufReader::new(f);
    use tokio::io::AsyncSeekExt;
    seekable.seek(std::io::SeekFrom::Start(start_byte)).await?;
    let mut lines = seekable.lines();

    let mut batch = Vec::with_capacity(deps.batch_size);
    let mut consumed_bytes = start_byte;
    while batch.len() < deps.batch_size {
        let Some(line) = lines.next_line().await? else { break; };
        consumed_bytes += line.len() as u64 + 1;
        let env: EventEnvelope = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => { warn!(error = %e, "skip malformed JSONL line"); continue; }
        };
        if let Some(sid) = session_id_of(&env.event) {
            match deps.cache.get(sid).unwrap_or(ShareDecision::Pending) {
                ShareDecision::Allowed => batch.push((line.len(), env)),
                ShareDecision::Pending => { break; /* hold; do not advance */ }
                ShareDecision::DeniedKeepLocal => { /* skip-ship, advance offset */ }
            }
        } else {
            // Events without session_id (rare; e.g. daemon-internal) — ship.
            batch.push((line.len(), env));
        }
    }

    if batch.is_empty() {
        return Ok(false);
    }

    let body = serde_json::to_vec(&serde_json::json!({
        "events": batch.iter().map(|(_, e)| e).collect::<Vec<_>>()
    }))?;
    post_batch(deps, client, &body).await?;

    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let new_off = SyncOffset { file: Some(filename), byte_offset: consumed_bytes };
    new_off.save(&deps.raw_dir)?;
    info!(shipped = batch.len(), "team_sync batch posted");
    Ok(true)
}

fn session_id_of(e: &IngestEvent) -> Option<SessionId> {
    use IngestEvent::*;
    match e {
        SessionStart { session_id, .. } => Some(*session_id),
        UserPrompt   { session_id, .. } => Some(*session_id),
        ToolCallEnd  { session_id, .. } => *session_id,
        FileDiff     { session_id, .. } => Some(*session_id),
        // Other variants — ship without holding.
        _ => None,
    }
}

fn select_jsonl_file(deps: &TeamSyncDeps, offset: &SyncOffset)
    -> Result<(Option<PathBuf>, u64)>
{
    // If offset points to a file, continue from it. If file rotated away, pick
    // the next-newest. If nothing exists, return (None, 0).
    if let Some(name) = offset.file.as_deref() {
        let p = deps.raw_dir.join(name);
        if p.exists() {
            // Did the file grow past offset.byte_offset?
            let len = std::fs::metadata(&p)?.len();
            if len > offset.byte_offset {
                return Ok((Some(p), offset.byte_offset));
            }
            // Check for a newer rotated file.
        }
    }
    // Pick the lexically latest *.jsonl.
    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in std::fs::read_dir(&deps.raw_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") { continue; }
        let mtime = entry.metadata()?.modified()?;
        if let Some((_, prev)) = &newest {
            if mtime <= *prev { continue; }
        }
        newest = Some((path, mtime));
    }
    let next_path = newest.map(|(p, _)| p);
    let start = if let Some(p) = &next_path {
        let same_file = offset.file.as_deref().map(|n| deps.raw_dir.join(n)) == Some(p.clone());
        if same_file { offset.byte_offset } else { 0 }
    } else { 0 };
    Ok((next_path, start))
}

async fn post_batch(deps: &TeamSyncDeps, client: &reqwest::Client, body: &[u8]) -> Result<()> {
    let url = format!("{}/v1/ingest", deps.team_cfg.server_url);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let mut attempt = 0u32;
    let mut backoff = Duration::from_secs(1);
    loop {
        attempt += 1;
        let claims = ProofClaims {
            htm: "POST".into(), htu: url.clone(), iat: now,
            jti: format!("jti-{}-{}", Instant::now().elapsed().as_nanos(),
                         uuid::Uuid::new_v4()),
            ath: token_hash_hex(&deps.team_cfg.device_token),
            bsh: body_hash_hex(body),
        };
        let proof = sign(&claims, &deps.signing_key);
        let resp = client.post(&url)
            .header("Authorization", format!("Bearer {}", deps.team_cfg.device_token))
            .header("X-Teramind-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body.to_vec())
            .send().await;
        match resp {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) if r.status().is_client_error() => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("ingest {status}: {text}"));
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                warn!(%status, body = %text, attempt, "ingest 5xx, retrying");
            }
            Err(e) => warn!(error = %e, attempt, "ingest network error, retrying"),
        }
        if attempt >= deps.max_attempts { return Err(anyhow::anyhow!("ingest exhausted retries")); }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}
```

- [ ] **Step 2: Register module**

In `crates/teramindd/src/services/mod.rs`, add `pub mod team_sync;`.

- [ ] **Step 3: Verify**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo build -p teramindd
cargo clippy -p teramindd --all-targets -- -D warnings
cargo test -p teramindd --test team_forwarder_e2e -- --test-threads=1
```

Expected: build silent, clippy silent, 1 PASS.

### Task 8.3: Commit

```bash
git add crates/teramindd/src/services/team_sync.rs \
        crates/teramindd/src/services/mod.rs \
        crates/teramindd/tests/team_forwarder_e2e.rs
git commit -m "feat(daemon): team_sync tail-JSONL forwarder with DPoP"
```

---

## Section 9 — Hold-and-backfill via DecisionCache

The E2E test in §8 already covered the `Allowed` happy path. Now add the privacy-gate test: `Pending` holds events, marker flip to `share=true` triggers backfill.

### Task 9.1: Failing test

**Files:**
- Create: `crates/teramindd/tests/team_share_decision_flow.rs`

- [ ] **Step 1: Write the test**

```rust
//! Privacy hold-and-backfill flow:
//! - Pending session: events accumulate in JSONL but do NOT ship.
//! - DecisionCache flips to Allowed (simulating MCP tool call).
//! - Forwarder next tick ships the held events.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use teramindd::services::team_sync::{TeamSync, TeamSyncDeps};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pending_holds_then_allowed_ships() -> anyhow::Result<()> {
    // boot server
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

    // redeem
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut OsRng);
    invites.create(&code.hash(), "alice@acme.dev", None, None,
                   OffsetDateTime::now_utc() + TDur::days(7)).await?;
    let r = reqwest::Client::new().post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "dev",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        })).send().await?;
    let body: serde_json::Value = r.json().await?;
    let team_cfg = TeamConfig {
        server_url: format!("http://{addr}"),
        user_email: "alice@acme.dev".into(),
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        device_token: body["device_token"].as_str().unwrap().into(),
        device_name: "dev".into(),
        redeemed_at: OffsetDateTime::now_utc(),
    };

    // Write Pending session to JSONL.
    let raw_dir = tempfile::tempdir()?;
    let jsonl = raw_dir.path().join("2026-05-17.jsonl");
    let sid = Uuid::new_v4();
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let envelope = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": started.format(&time::format_description::well_known::Rfc3339)?,
        "event": { "type": "session_start", "session_id": sid.to_string(),
                   "agent_kind": "claude_code", "cwd": "/proj",
                   "os": "linux", "hostname": "h", "user_login": "u",
                   "git_head": null, "git_branch": null, "agent_session_id": null }
    });
    std::fs::write(&jsonl, format!("{}\n", serde_json::to_string(&envelope)?))?;

    let cache = DecisionCache::new();
    cache.set_initial(SessionId(sid), ShareDecision::Pending);

    let _forwarder = TeamSync::spawn(TeamSyncDeps {
        team_cfg: Arc::new(team_cfg),
        signing_key: Arc::new(sk),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8,
        max_attempts: 3,
    });

    // Wait — should NOT arrive.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (n0,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
        .bind(sid).fetch_one(pool.pg()).await?;
    assert_eq!(n0, 0, "Pending must NOT ship");

    // Flip to Allowed.
    cache.set(SessionId(sid), ShareDecision::Allowed);

    // Wait — should arrive.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
            .bind(sid).fetch_one(pool.pg()).await?;
        if n == 1 { break; }
    }
    let (n1,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
        .bind(sid).fetch_one(pool.pg()).await?;
    assert_eq!(n1, 1, "Allowed flip must trigger backfill");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test -p teramindd --test team_share_decision_flow -- --test-threads=1
```

Expected: PASS. (The forwarder's existing tick loop already re-checks the cache on each cycle, so no implementation change is needed beyond what §8 already wrote.)

### Task 9.2: Commit

```bash
git add crates/teramindd/tests/team_share_decision_flow.rs
git commit -m "test(daemon): hold-and-backfill privacy gate"
```

---

## Section 10 — SessionStart hook share-prompt injection

The hook is what triggers the agent to ask the user. When `SessionStart` fires for a project that has no `.teramind/team-share.toml` marker (and only when team mode is configured), the hook injects a notice into Claude's context so the agent prompts the user in the next turn.

### Task 10.1: Locate the hook's SessionStart path

- [ ] **Step 1: Inspect**

Run: `grep -n 'SessionStart\|session_start' crates/teramind-hook/src/translate.rs crates/teramind-hook/src/main.rs | head`

Identify the function that handles SessionStart events. There's an existing path that emits an auto-recall digest; we add the share-prompt notice alongside it.

### Task 10.2: Inject the notice

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs` (or wherever SessionStart context-injection lives)

- [ ] **Step 1: Add the team-share branch**

Add a helper:

```rust
fn maybe_share_prompt(cwd: &std::path::Path) -> Option<String> {
    let team_toml = teramind_core::team::default_config_dir().join("team.toml");
    if !team_toml.exists() {
        return None; // not team-mode
    }
    let home = std::env::var("HOME").ok().map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"));
    if teramindd::services::team_share::find_marker(cwd, &home).is_some() {
        return None; // already decided
    }
    Some(format!(
        "⚠️ This project at `{}` has no Teramind team-sharing preference set. \
         Please ask the user once: \"Share captures from this project with the team?\" \
         Then call `mcp__teramind__team_share_set(scope: 'project', share: true | false)` \
         to record their answer. Until then, captures stay local-only.",
        cwd.display()
    ))
}
```

Where the existing SessionStart hook concatenates context strings (e.g., the auto-recall digest), append the share prompt:

```rust
let mut chunks: Vec<String> = vec![];
// …existing auto-recall digest…
if let Some(notice) = maybe_share_prompt(&cwd) {
    chunks.push(notice);
}
// …existing print to stdout for hook context injection…
```

(Adapt to the existing chunk structure — the goal is one extra string appended.)

### Task 10.3: Caveat on dep direction

`teramind-hook` doesn't normally depend on `teramindd`. If `teramindd::services::team_share` is the only thing needed, **inline a copy** of `find_marker` into `teramind-hook` to avoid the dep cycle. The marker walker is ~30 lines; small enough to duplicate. Add it as `crates/teramind-hook/src/team_share.rs` and `pub mod team_share;` in `lib.rs`.

OR cleaner: move `team_share` to `teramind-core::team_share` so both crates can depend on it. Pick whichever feels more natural in the codebase. The plan favors **moving to `teramind-core::team_share`** for a single source of truth.

If you move it: also update `crates/teramindd/src/services/team_share.rs` to re-export from core:

```rust
pub use teramind_core::team_share::{find_marker, write_marker_at_cwd, ShareMarker};
```

…or delete the daemon copy and switch imports to `teramind_core::team_share`.

### Task 10.4: Test the hook injects the notice

**Files:**
- Create or extend: `crates/teramind-hook/tests/team_share_prompt.rs`

- [ ] **Step 1**

```rust
//! When team.toml exists AND no marker exists in cwd, the SessionStart hook
//! must emit the share-prompt notice in its stdout context.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use teramind_core::team::{save_signing_key, TeamConfig};

#[test]
fn session_start_with_team_mode_and_no_marker_emits_prompt() {
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());

    // Make team-mode look configured.
    let mut seed = [0u8; 32]; OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let team_dir = cfg_dir.path().join("teramind");
    std::fs::create_dir_all(&team_dir).unwrap();
    let cfg = TeamConfig {
        server_url: "https://srv".into(),
        user_email: "alice@acme.dev".into(),
        user_id: uuid::Uuid::new_v4().to_string(),
        device_id: uuid::Uuid::new_v4().to_string(),
        device_token: "tmd_v1_X".into(),
        device_name: "x".into(),
        redeemed_at: time::OffsetDateTime::now_utc(),
    };
    cfg.save(&team_dir.join("team.toml")).unwrap();
    save_signing_key(&team_dir.join("team-key"), &sk).unwrap();

    // Project dir with NO marker.
    let proj = tempfile::tempdir().unwrap();
    let out = teramind_hook::session_start_context(proj.path());
    assert!(out.contains("Share captures from this project"),
            "share prompt must appear in hook output; got: {out}");
}
```

`teramind_hook::session_start_context` is a helper you may need to extract into the hook crate's lib for testability (returns the concatenated context string). If the existing code writes directly to stdout, refactor to return the string and let the binary path print it.

### Task 10.5: Verify + commit

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p teramind-hook --test team_share_prompt
git add crates/teramind-core crates/teramind-hook crates/teramindd
git commit -m "feat(hook): inject team-share prompt on SessionStart"
```

---

## Section 11 — `mcp__teramind__team_share_set` MCP tool

### Task 11.1: Add the tool

**Files:**
- Modify: `crates/teramind-mcp/src/server.rs`

- [ ] **Step 1: Inspect existing tools**

Run: `grep -n 'tool_\|register_tool\|McpTool' crates/teramind-mcp/src/server.rs | head -20`

Identify how existing tools (`search`, `recall`, `save_skill`, `wiki`) are declared. Follow the same pattern.

- [ ] **Step 2: Define the tool body**

Add a new tool with this contract:

- Name: `mcp__teramind__team_share_set`
- Args: `scope: string` ("project" in v1.0; "session" reserved), `share: bool`
- Reads `cwd` from environment (`std::env::current_dir()`) at tool-call time.
- Optionally reads `session_id` from the MCP request env if available (look at how the wiki tool does this).
- Sends `Request::TeamShareSet { session_id, cwd, scope, share }` over the existing IPC client (`IpcClient`).
- Returns `{"ok": true}` on success, `{"ok": false, "error": "…"}` otherwise.

Use the same `RpcTransport` / `IpcClient` machinery `mcp__teramind__search` uses today. The exact code shape depends on the existing tool registration mechanism; the contract above is the spec.

### Task 11.2: Test the round trip

**Files:**
- Create or extend: `crates/teramind-mcp/tests/team_share_set.rs`

- [ ] **Step 1: Mock daemon + assert IPC dispatch**

Start a unit-style test: launch a mock IPC server that records the requests it sees, fire the MCP tool via `teramind-mcp`'s server lib, assert the recorded request is `Request::TeamShareSet { cwd, scope: "project", share: true, .. }`.

If the existing test pattern for `mcp__teramind__search` already provides a mock daemon, reuse it.

- [ ] **Step 2: Run**

```bash
cargo test -p teramind-mcp --test team_share_set
```

Expected: PASS.

### Task 11.3: Commit

```bash
git add crates/teramind-mcp
git commit -m "feat(mcp): mcp__teramind__team_share_set tool"
```

---

## Section 12 — Wire `team_sync` into the daemon

### Task 12.1: Spawn the forwarder on startup

**Files:**
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Read team.toml at startup**

Near the top of `App::run`, after `Paths::resolve()`, add:

```rust
let team_cfg_path = paths.config_dir.join("team.toml");
let team_mode = if team_cfg_path.exists() {
    let cfg = teramind_core::team::TeamConfig::load(&team_cfg_path)
        .context("load team.toml")?;
    let key = teramind_core::team::load_signing_key(&paths.config_dir.join("team-key"))
        .context("load team-key")?;
    Some((std::sync::Arc::new(cfg), std::sync::Arc::new(key)))
} else { None };
```

- [ ] **Step 2: Spawn the forwarder if team-mode is configured**

Before the IPC server is constructed, add:

```rust
let decision_cache = crate::services::decision_cache::DecisionCache::new();
let _forwarder = team_mode.as_ref().map(|(cfg, sk)| {
    crate::services::team_sync::TeamSync::spawn(
        crate::services::team_sync::TeamSyncDeps {
            team_cfg: cfg.clone(),
            signing_key: sk.clone(),
            raw_dir: paths.raw_dir.clone(),
            cache: decision_cache.clone(),
            poll_interval: std::time::Duration::from_secs(1),
            batch_size: 32,
            max_attempts: 5,
        }
    )
});
```

- [ ] **Step 3: Build a real `TeamShareSetter` for the IPC handler**

In `app.rs`, construct a concrete writer that writes the marker file and updates the cache. Put the type in `crates/teramindd/src/services/team_share.rs`:

```rust
pub struct DaemonTeamShareSetter {
    pub cache: std::sync::Arc<crate::services::decision_cache::DecisionCache>,
    pub user_email: String,
}

#[async_trait::async_trait]
impl crate::services::ipc_server::TeamShareSetter for DaemonTeamShareSetter {
    async fn write_and_signal(&self, cwd: &std::path::Path,
                              session_id: Option<teramind_core::ids::SessionId>,
                              share: bool, _set_by: &str) -> anyhow::Result<()> {
        let marker = ShareMarker {
            share,
            set_by: self.user_email.clone(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(cwd, &marker)?;
        if let Some(sid) = session_id {
            let new = if share {
                crate::services::decision_cache::ShareDecision::Allowed
            } else {
                crate::services::decision_cache::ShareDecision::DeniedKeepLocal
            };
            self.cache.set(sid, new);
        }
        Ok(())
    }
}
```

Wire it into the IPC handler:

```rust
let team_share_writer = team_mode.as_ref().map(|(cfg, _)| {
    std::sync::Arc::new(crate::services::team_share::DaemonTeamShareSetter {
        cache: decision_cache.clone(),
        user_email: cfg.user_email.clone(),
    }) as std::sync::Arc<dyn crate::services::ipc_server::TeamShareSetter>
});
```

…and inject it + `decision_cache` into the `DaemonIpcHandler` struct literal.

- [ ] **Step 4: Build + smoke**

```bash
cargo build -p teramindd
cargo clippy -p teramindd --all-targets -- -D warnings
```

Both silent.

### Task 12.2: Commit

```bash
git add crates/teramindd/src/app.rs \
        crates/teramindd/src/services/team_share.rs \
        crates/teramindd/src/services/ipc_server.rs
git commit -m "feat(daemon): wire team_sync + DecisionCache into app startup"
```

---

## Section 13 — Full doctor team-mode rendering

Plan I added a one-line placeholder. Now render full health.

### Task 13.1: Replace the team-mode block in doctor

**Files:**
- Modify: `crates/teramind/src/commands/doctor.rs`

- [ ] **Step 1: Replace**

Find the existing `team mode:` block (introduced in Plan I §19). Replace with:

```rust
let team_toml = paths.config_dir.join("team.toml");
let team_key  = paths.config_dir.join("team-key");
match teramind_core::team::TeamConfig::load(&team_toml) {
    Ok(cfg) => {
        println!("team mode:   enabled ({})", cfg.server_url);
        println!("user/device: {} / {}", cfg.user_email, cfg.device_name);
        match teramind_core::team::load_signing_key(&team_key) {
            Ok(_) => println!("auth proof:  ed25519 (key at {}, mode 0600 ✓)", team_key.display()),
            Err(e) => println!("auth proof:  ✗ {}", e),
        }
        // Forwarder backlog comes from teramindd Status response — we already
        // fetch it earlier in doctor for embedding/summary surfaces. Reuse it.
        // (Read from `status.team_sync_*` fields if present; render "—" otherwise.)
    }
    Err(e) if team_toml.exists() => {
        println!("team mode:   ✗ team.toml present but unreadable: {e}");
    }
    Err(_) => {
        println!("team mode:   not configured (run `teramind init --team --server=… --invite=…` to opt in)");
    }
}
```

- [ ] **Step 2 (optional, defer if scope grows):** add `team_sync_*` fields to `StatusReport` and surface throughput. If skipped, leave a TODO referencing Plan J §13.5 for v1.1.

### Task 13.2: Verify

```bash
cargo build -p teramind
cargo clippy -p teramind --all-targets -- -D warnings
./target/debug/teramind doctor 2>&1 | grep -A2 'team mode:'
```

### Task 13.3: Commit

```bash
git add crates/teramind/src/commands/doctor.rs
git commit -m "feat(cli): doctor renders full team-mode health"
```

---

## Section 14 — Final workspace check

### Task 14.1: Run everything

```bash
export GITHUB_TOKEN=$(gh auth token)
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plan I baseline: 300 tests. This plan adds:
- §1 dpop: 1 test
- §2 team: 3 tests
- §3 init_team: 1 test
- §4 team_share: 4 tests
- §6 decision_cache: 3 tests
- §7 sync_offset: 3 tests
- §8 team_forwarder_e2e: 1 test
- §9 team_share_decision_flow: 1 test
- §10 team_share_prompt: 1 test
- §11 team_share_set MCP: 1 test

Total new: ~19 tests. Expected total: ~319.

### Task 14.2: Manual smoke (optional)

1. `cargo build --release -p teramind-sync-server -p teramindd -p teramind`
2. Start a sync server locally (in docker compose or via the binary with `--insecure-allow-http`).
3. `teramind-sync-server invite create --email me@local` → copy the `TM-…` code.
4. `teramind init --team --server=http://localhost:8443 --invite=TM-…`.
5. `teramind start` (or whatever the daemon-start command is).
6. Run a real Claude session in a project — observe that the hook injects the share-prompt notice.
7. Have Claude call `mcp__teramind__team_share_set(scope: 'project', share: true)`.
8. Continue the session; verify rows appear server-side: `psql ... -c "SELECT count(*) FROM sessions WHERE user_id IS NOT NULL;"`.

### Task 14.3: PR prep (do NOT push without explicit consent)

```bash
git log --oneline main..HEAD
git diff --stat main..HEAD | tail -10
```

Report the final SHA and the commit log to the controller. The controller decides whether to push + open a PR.

---

## Spec coverage matrix

| Spec section | Plan J addresses | Notes |
|---|---|---|
| §2.1 In-scope — `teramind init --team` | §3 (init_team subcommand) | — |
| §2.1 In-scope — JSONL forwarder + decision cache | §6 (cache), §7 (offset), §8 (forwarder), §9 (hold-and-backfill) | — |
| §2.1 In-scope — per-project marker | §4 (walker) | — |
| §2.1 In-scope — agent-prompted privacy + mcp__teramind__team_share_set | §10 (hook prompt), §11 (MCP tool), §6 (TeamShareSet IPC) | — |
| §2.1 In-scope — read-path fallback | — | Plan K |
| §2.1 In-scope — MCP proxy mechanics | — | Plan K |
| §2.1 In-scope — live propagation | — | Plan L |
| §5.2 Device redemption | §3 | Plan I shipped the server side |
| §5.3 Per-request auth (DPoP) | §1 (extract sign helpers), §8 (forwarder uses sign) | — |
| §5.4 Defense properties | §2 (0600 enforcement), §8 (signing only happens with key) | Plan I tested the server-side reject |
| §5.5 Doctor surfaces | §13 (full team-mode rendering) | — |
| §6.1 Tail-forwarder loop | §8 | — |
| §6.2 Per-session decision cache | §6 | — |
| §6.3 Agent-driven privacy prompt | §10, §11 | — |
| §6.4 Backfill on consent | §9 | — |
| §6.5 Server-side /v1/ingest | — | Plan I |
| §6.6 Doctor forwarder surfaces | §13 (partial — full throughput is v1.1) | — |
| §10 Testing — L1/L2 | §1–§7 unit tests | — |
| §10.3 L3 multi-daemon harness | — | Plan K (needs MCP proxy first) |
