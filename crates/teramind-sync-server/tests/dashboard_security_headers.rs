//! Dashboard §8 — /dashboard/* responses carry a Content-Security-Policy header
//! with default-src, connect-src, and frame-ancestors directives.

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
async fn dashboard_index_has_csp_header() -> anyhow::Result<()> {
    let addr = boot().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/dashboard"))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let csp = r
        .headers()
        .get("content-security-policy")
        .expect("CSP header must be present on /dashboard responses")
        .to_str()?
        .to_string();
    assert!(
        csp.contains("default-src"),
        "CSP must include default-src directive: {csp}"
    );
    assert!(
        csp.contains("connect-src"),
        "CSP must include connect-src directive: {csp}"
    );
    assert!(
        csp.contains("frame-ancestors"),
        "CSP must include frame-ancestors directive: {csp}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashboard_subpath_also_carries_csp() -> anyhow::Result<()> {
    let addr = boot().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/dashboard/some-spa-route"))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let csp = r
        .headers()
        .get("content-security-policy")
        .expect("CSP must be set on dashboard subpaths too")
        .to_str()?
        .to_string();
    assert!(csp.contains("frame-ancestors"));
    Ok(())
}
