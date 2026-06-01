//! Dashboard §5 — GET /admin/quality/config returns
//! { enabled, cron, last_run_at, next_run_at } (plus baselines for context).

use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

fn admin_cfg(password: &str) -> AdminConfig {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string();
    AdminConfig {
        admin_password_hash: hash,
        admin_session_secret: "ab".repeat(32),
        admin_session_ttl_hours: 12,
        event_log_retention_days: 90,
    }
}

async fn boot() -> anyhow::Result<(SocketAddr, String)> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(admin_cfg("hunter2hunter2")),
        quality: Some(QualityConfig {
            enabled: true,
            cron: Some("0 2 * * *".into()),
            baselines: vec!["lexical".into()],
            eval_binary: "teramind-search-eval".into(),
        }),
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
async fn config_returns_documented_keys() -> anyhow::Result<()> {
    let (addr, cookie) = boot().await?;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/quality/config"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;

    // All four documented keys MUST be present (next_run_at / last_run_at may be null).
    for key in &["enabled", "cron", "last_run_at", "next_run_at"] {
        assert!(
            body.get(*key).is_some(),
            "config response must include `{key}` key (got: {body})"
        );
    }
    assert_eq!(body["enabled"], true);
    assert_eq!(body["cron"], "0 2 * * *");
    // last_run_at is null because no run has been recorded yet.
    assert!(body["last_run_at"].is_null());
    // next_run_at is computed from the cron expression — must be a non-empty string.
    let next = body["next_run_at"]
        .as_str()
        .expect("next_run_at must be a string when scheduler is enabled");
    assert!(!next.is_empty());
    Ok(())
}
