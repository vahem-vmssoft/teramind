//! Dashboard §2 — when [admin] config block is absent, both /admin/* and
//! /dashboard/* are NOT mounted on the router and therefore return 404.

use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

async fn boot_without_admin() -> anyhow::Result<SocketAddr> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None, // <- no admin block
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
async fn dashboard_root_is_404_when_admin_absent() -> anyhow::Result<()> {
    let addr = boot_without_admin().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/dashboard"))
        .send()
        .await?;
    assert_eq!(
        r.status(),
        404,
        "dashboard root must 404 when [admin] is absent"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_health_is_404_when_admin_absent() -> anyhow::Result<()> {
    let addr = boot_without_admin().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/health"))
        .send()
        .await?;
    assert_eq!(
        r.status(),
        404,
        "/admin/health must 404 when [admin] is absent"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_login_is_404_when_admin_absent() -> anyhow::Result<()> {
    let addr = boot_without_admin().await?;
    // Login is a POST in normal use but a GET also reveals the route — both
    // should 404 because the entire admin router is omitted.
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "anything" }))
        .send()
        .await?;
    assert_eq!(r.status(), 404);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v1_health_still_works_without_admin() -> anyhow::Result<()> {
    // Sanity check: gating admin must not affect the public /v1 routes.
    let addr = boot_without_admin().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/v1/health"))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    Ok(())
}
