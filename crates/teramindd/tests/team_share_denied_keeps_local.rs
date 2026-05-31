//! team-sync §2.1: DeniedKeepLocal sessions keep their events in JSONL but
//! the forwarder NEVER ships them to the central server.

use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_db::repos::InviteRepo;
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use teramindd::services::team_sync::{TeamSync, TeamSyncDeps};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn denied_keeps_local_never_ships() -> anyhow::Result<()> {
    // Boot a real sync server so we can confirm zero rows arrive.
    let pool = teramind_db::testing::fresh_pool().await?;
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

    // Redeem an invite to get a valid team config.
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut rand::rng());
    invites
        .create(
            &code.hash(),
            "bob@acme.dev",
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
        user_email: "bob@acme.dev".into(),
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        device_token: body["device_token"].as_str().unwrap().into(),
        device_name: "dev".into(),
        redeemed_at: OffsetDateTime::now_utc(),
    };

    // Write a session_start + user_prompt to JSONL for a session marked DeniedKeepLocal.
    let raw_dir = tempfile::tempdir()?;
    let jsonl_path = raw_dir.path().join("2026-05-31.jsonl");
    let sid = Uuid::new_v4();
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ts = started.format(&time::format_description::well_known::Rfc3339)?;
    let start_env = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": ts,
        "event": { "type": "session_start", "session_id": sid.to_string(),
                   "agent_kind": "claude_code", "cwd": "/proj",
                   "os": "linux", "hostname": "h", "user_login": "u",
                   "git_head": null, "git_branch": null, "agent_session_id": null }
    });
    let prompt_env = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": ts,
        "event": { "type": "user_prompt", "session_id": sid.to_string(),
                   "turn_ordinal": 0, "prompt": "denied secret content", "turn_id": null }
    });
    let body = format!(
        "{}\n{}\n",
        serde_json::to_string(&start_env)?,
        serde_json::to_string(&prompt_env)?
    );
    std::fs::write(&jsonl_path, &body)?;

    // Decision is DeniedKeepLocal — forwarder must skip events for this session.
    let cache = DecisionCache::new();
    cache.set_initial(SessionId(sid), ShareDecision::DeniedKeepLocal);

    let _forwarder = TeamSync::spawn(TeamSyncDeps {
        team_cfg: Arc::new(team_cfg),
        signing_key: Arc::new(sk),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8,
        max_attempts: 3,
    });

    // Wait a generous window — nothing should ship.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let (server_n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id = $1")
        .bind(sid)
        .fetch_one(pool.pg())
        .await?;
    assert_eq!(server_n, 0, "DeniedKeepLocal must never ship to server");

    // JSONL file is still intact on disk.
    let on_disk = std::fs::read_to_string(&jsonl_path)?;
    assert!(on_disk.contains("denied secret content"));
    assert!(on_disk.contains(&sid.to_string()));

    Ok(())
}
