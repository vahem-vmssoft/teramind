//! Dashboard §5.8 — GET /admin/health returns the documented field set:
//! db, broadcast_subscribers, codifier_backlog, team_sync, quality_scheduler,
//! ingest, and uptime_seconds.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHasher};
use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

async fn boot() -> anyhow::Result<(SocketAddr, String)> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(b"hunter2hunter2", &salt)
        .unwrap()
        .to_string();
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(AdminConfig {
            admin_password_hash: hash,
            admin_session_secret: "ab".repeat(32),
            admin_session_ttl_hours: 12,
            event_log_retention_days: 90,
        }),
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let login = reqwest::Client::new()
        .post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" }))
        .send()
        .await?;
    let cookie = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()?
        .split(';')
        .next()
        .unwrap()
        .to_string();
    Ok((addr, cookie))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_health_carries_all_documented_fields() -> anyhow::Result<()> {
    let (addr, cookie) = boot().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/health"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let obj = body.as_object().expect("response must be a JSON object");
    for key in [
        "db",
        "broadcast_subscribers",
        "codifier_backlog",
        "team_sync",
        "quality_scheduler",
        "ingest",
        "uptime_seconds",
    ] {
        assert!(
            obj.contains_key(key),
            "missing field `{key}` in /admin/health response: {body}"
        );
    }
    Ok(())
}
