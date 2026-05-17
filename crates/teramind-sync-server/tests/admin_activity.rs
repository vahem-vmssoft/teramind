//! /admin/activity (HTTP) + /admin/events (WS).

use std::net::SocketAddr;
use teramind_core::team_event::TeamEvent;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use uuid::Uuid;
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};

fn admin_cfg_with_password(password: &str) -> AdminConfig {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt).unwrap().to_string();
    AdminConfig {
        admin_password_hash: hash,
        admin_session_secret: "ab".repeat(32),
        admin_session_ttl_hours: 12,
        event_log_retention_days: 90,
    }
}

async fn boot() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, AppState, String)> {
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
        admin: Some(admin_cfg_with_password("hunter2hunter2")),
        quality: None,
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .await.unwrap();
    });
    let login = reqwest::Client::new().post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" })).send().await?;
    let cookie = login.headers().get("set-cookie").unwrap().to_str()?
        .split(';').next().unwrap().to_string();
    Ok((dir, sup, addr, state, cookie))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn activity_returns_recent_rows() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    state.event_log.insert("skill_saved", None, None, serde_json::json!({"name":"x"})).await?;
    state.event_log.insert("session_ended", None, Some("/proj".into()), serde_json::json!({})).await?;

    let r = reqwest::Client::new().get(format!("http://{addr}/admin/activity?limit=10"))
        .header("Cookie", &cookie).send().await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["events"].as_array().unwrap().len(), 2);

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_subscriber_receives_bus_event() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    let req = Request::builder()
        .uri(format!("ws://{addr}/admin/events"))
        .header("Host", addr.to_string())
        .header("Cookie", &cookie)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())?;
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    let (_w, mut r) = ws.split();

    // Eat hello.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), r.next()).await?.unwrap()?;

    let _ = state.bus.send(TeamEvent::SkillSaved {
        skill_id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        name: "test".into(),
        ts: time::OffsetDateTime::now_utc(),
    });

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), r.next()).await?.unwrap()?;
    if let Message::Text(t) = msg {
        let evt: TeamEvent = serde_json::from_str(&t)?;
        match evt {
            TeamEvent::SkillSaved { name, .. } => assert_eq!(name, "test"),
            _ => panic!("unexpected"),
        }
    } else { panic!("expected text"); }

    sup.shutdown().await?; Ok(())
}
