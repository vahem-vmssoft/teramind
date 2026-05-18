//! Privacy hold-and-backfill flow:
//! - Pending session: events accumulate in JSONL but do NOT ship.
//! - DecisionCache flips to Allowed (simulating MCP tool call).
//! - Forwarder next tick ships the held events.

use ed25519_dalek::SigningKey;
use rand::RngExt;
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
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // redeem
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut rand::rng());
    invites
        .create(
            &code.hash(),
            "alice@acme.dev",
            None,
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await?;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "dev",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        }))
        .send()
        .await?;
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
        .bind(sid)
        .fetch_one(pool.pg())
        .await?;
    assert_eq!(n0, 0, "Pending must NOT ship");

    // Flip to Allowed.
    cache.set(SessionId(sid), ShareDecision::Allowed);

    // Wait — should arrive.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
            .bind(sid)
            .fetch_one(pool.pg())
            .await?;
        if n == 1 {
            break;
        }
    }
    let (n1,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
        .bind(sid)
        .fetch_one(pool.pg())
        .await?;
    assert_eq!(n1, 1, "Allowed flip must trigger backfill");

    sup.shutdown().await?;
    Ok(())
}
