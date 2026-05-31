//! Dashboard §5 — GET /admin/version is reachable WITHOUT a session cookie
//! and returns { "version": "<crate-version>" }. This is the only admin route
//! the SPA can call before login (to render the footer on the login screen).

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHasher};
use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

async fn boot() -> anyhow::Result<SocketAddr> {
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
    Ok(addr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn version_is_reachable_without_session_cookie() -> anyhow::Result<()> {
    let addr = boot().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/version"))
        .send()
        .await?;
    assert_eq!(
        r.status(),
        200,
        "/admin/version must be public (no auth required)"
    );
    let body: serde_json::Value = r.json().await?;
    let v = body
        .get("version")
        .and_then(|v| v.as_str())
        .expect("response must carry a `version` string");
    assert!(!v.is_empty(), "version string must not be empty");
    Ok(())
}
