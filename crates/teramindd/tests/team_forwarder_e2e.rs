//! End-to-end forwarder integration test:
//! - Spin up the central sync server against an embedded PG.
//! - Redeem an invite to get a real team.toml + team-key.
//! - Start the local forwarder pointing at that server.
//! - Append a SessionStart envelope to a JSONL file.
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
