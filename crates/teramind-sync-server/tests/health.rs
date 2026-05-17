//! Spins up the server against an embedded PG and hits /v1/health + /v1/version.

use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_returns_ok_when_db_up() -> anyhow::Result<()> {
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
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let resp = reqwest::get(format!("http://{addr}/v1/health")).await?;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["status"], "ok");

    let ver = reqwest::get(format!("http://{addr}/v1/version"))
        .await?
        .json::<serde_json::Value>()
        .await?;
    assert_eq!(ver["version"], env!("CARGO_PKG_VERSION"));

    sup.shutdown().await?;
    Ok(())
}
