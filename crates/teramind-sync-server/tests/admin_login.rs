//! /admin/login: 200+cookie on correct password; 401 on wrong; 429 after 5 failures.

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
async fn login_succeeds_with_correct_password() -> anyhow::Result<()> {
    let addr = boot("hunter2hunter2").await?;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "hunter2hunter2" }))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let set_cookie = r.headers().get("set-cookie").unwrap().to_str()?.to_string();
    assert!(set_cookie.starts_with("tmd_admin="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_fails_with_wrong_password() -> anyhow::Result<()> {
    let addr = boot("hunter2hunter2").await?;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "wrong" }))
        .send()
        .await?;
    assert_eq!(r.status(), 401);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_limits_after_five_failures() -> anyhow::Result<()> {
    let addr = boot("hunter2hunter2").await?;
    for _ in 0..5 {
        let r = reqwest::Client::new()
            .post(format!("http://{addr}/admin/login"))
            .json(&serde_json::json!({ "password": "wrong" }))
            .send()
            .await?;
        assert_eq!(r.status(), 401);
    }
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/login"))
        .json(&serde_json::json!({ "password": "wrong" }))
        .send()
        .await?;
    assert_eq!(r.status(), 429);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn me_requires_cookie() -> anyhow::Result<()> {
    let addr = boot("hunter2hunter2").await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/me"))
        .send()
        .await?;
    assert_eq!(r.status(), 401);

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
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/me"))
        .header("Cookie", cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["admin"], true);

    Ok(())
}
