//! Dashboard §4 — POST /admin/logout returns 200 with a Set-Cookie header
//! that clears the tmd_admin session (Max-Age=0 or Expires in the past).

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHasher};
use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

async fn boot(password: &str) -> anyhow::Result<SocketAddr> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
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
async fn logout_clears_session_cookie() -> anyhow::Result<()> {
    let addr = boot("hunter2hunter2").await?;

    let login = reqwest::Client::new()
        .post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" }))
        .send()
        .await?;
    assert_eq!(login.status(), 200);
    let cookie = login
        .headers()
        .get("set-cookie")
        .expect("login should issue a session cookie")
        .to_str()?
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/logout"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);

    let set_cookie = r
        .headers()
        .get("set-cookie")
        .expect("logout must emit a Set-Cookie that clears tmd_admin")
        .to_str()?
        .to_string();
    assert!(
        set_cookie.contains("tmd_admin"),
        "logout Set-Cookie should target tmd_admin: {set_cookie}"
    );
    let clears = set_cookie.contains("Max-Age=0")
        || set_cookie.contains("max-age=0")
        || set_cookie.to_ascii_lowercase().contains("expires=thu, 01 jan 1970");
    assert!(
        clears,
        "logout Set-Cookie should clear the session (Max-Age=0 or past Expires): {set_cookie}"
    );
    Ok(())
}
