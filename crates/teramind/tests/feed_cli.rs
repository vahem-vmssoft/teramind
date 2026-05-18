//! Smoke test for `teramind feed`: spin up server inline, write team.toml
//! to a temp XDG_CONFIG_HOME, publish a TeamEvent on the server bus, and
//! call `feed::run(false, 0)` directly — assert it returns Ok within 2s.

use base64::Engine;
use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use std::time::Duration;
use teramind_core::team::{save_signing_key, TeamConfig};
use teramind_core::team_event::TeamEvent;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn feed_prints_one_event_then_exits() -> anyhow::Result<()> {
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
    let server_bus = state.bus.clone();
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Create an invite and redeem it to get a device token.
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
            "invite_code": code.as_str(),
            "device_name": "dev",
            "device_public_key_b64": base64::engine::general_purpose::STANDARD.encode(&pk),
        }))
        .send()
        .await?;
    let body: serde_json::Value = r.json().await?;

    // Write team.toml + team-key in a temp XDG_CONFIG_HOME.
    let cfg_dir = tempfile::tempdir()?;
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());
    let team_dir = cfg_dir.path().join("teramind");
    std::fs::create_dir_all(&team_dir)?;
    let team_cfg = TeamConfig {
        server_url: format!("http://{addr}"),
        user_email: "alice@acme.dev".into(),
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        device_token: body["device_token"].as_str().unwrap().into(),
        device_name: "dev".into(),
        redeemed_at: OffsetDateTime::now_utc(),
    };
    team_cfg.save(&team_dir.join("team.toml"))?;
    save_signing_key(&team_dir.join("team-key"), &sk)?;

    // Spawn feed::run(false, 0); it connects, receives hello, waits for a real event, prints, returns Ok.
    let feed_handle = tokio::spawn(async move { teramind::commands::feed::run(false, 0).await });

    // Give the WS handshake a moment.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Publish a TeamEvent on the server bus.
    let _ = server_bus.send(TeamEvent::SessionEnded {
        session_id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        cwd: "/x".into(),
        ts: OffsetDateTime::now_utc(),
    });

    // feed::run should return Ok within 2s.
    let result = tokio::time::timeout(Duration::from_secs(2), feed_handle).await??;
    assert!(result.is_ok(), "feed::run must succeed; got {:?}", result);

    sup.shutdown().await?;
    Ok(())
}
