//! E2E: spin up server, redeem, start TeamEvents in this test process
//! pointing at the server. Publish a TeamEvent on the server bus directly.
//! Assert the local broadcast receives it.

use base64::Engine;
use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::team::TeamConfig;
use teramind_core::team_event::TeamEvent;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use teramindd::services::team_events::{TeamEvents, TeamEventsDeps};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_subscriber_receives_server_bus_event() -> anyhow::Result<()> {
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

    // Redeem.
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
            "device_public_key_b64": base64::engine::general_purpose::STANDARD.encode(&pk),
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

    // Local bus + subscriber.
    let (local_bus, mut local_rx) = tokio::sync::broadcast::channel::<TeamEvent>(16);
    let _sub = TeamEvents::spawn(TeamEventsDeps {
        team_cfg: Arc::new(team_cfg),
        signing_key: Arc::new(sk),
        local_bus,
    });

    // Give the WS handshake a moment.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Publish on the server.
    let sid = Uuid::new_v4();
    let _ = server_bus.send(TeamEvent::SessionEnded {
        session_id: sid,
        user_id: Uuid::new_v4(),
        cwd: "/x".into(),
        ts: OffsetDateTime::now_utc(),
    });

    // Receive locally.
    let recv = tokio::time::timeout(Duration::from_secs(3), local_rx.recv()).await??;
    match recv {
        TeamEvent::SessionEnded { session_id, .. } => assert_eq!(session_id, sid),
        other => panic!("unexpected: {other:?}"),
    }

    sup.shutdown().await?;
    Ok(())
}
