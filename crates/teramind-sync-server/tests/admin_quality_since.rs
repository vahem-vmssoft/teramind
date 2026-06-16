//! Dashboard §5 — GET /admin/quality?since=<RFC3339>&limit= returns rows
//! strictly newer than `since`, ordered by ran_at descending.

use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::format_description::well_known::Rfc3339;

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

async fn boot() -> anyhow::Result<(SocketAddr, AppState, String)> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(admin_cfg("hunter2hunter2")),
        quality: None,
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state.clone());
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
    Ok((addr, state, cookie))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn since_filters_to_newer_rows() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    // Insert older row.
    state
        .quality
        .insert(
            "lexical",
            None,
            0.4,
            0.3,
            0.2,
            0.1,
            0.5,
            50.0,
            150.0,
            10,
            100,
            serde_json::json!({}),
            serde_json::json!({}),
            "manual",
        )
        .await?;
    // Capture a cutoff that sits strictly between the two inserts.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let cutoff = time::OffsetDateTime::now_utc();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    // Insert newer row.
    state
        .quality
        .insert(
            "lexical",
            None,
            0.8,
            0.7,
            0.6,
            0.5,
            0.9,
            30.0,
            100.0,
            10,
            100,
            serde_json::json!({}),
            serde_json::json!({}),
            "manual",
        )
        .await?;

    let since = cutoff.format(&Rfc3339)?;
    let r = reqwest::Client::new()
        .get(format!(
            "http://{addr}/admin/quality?since={since}&limit=10"
        ))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let runs = body["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1, "only the newer row should be in-window");
    let n = runs[0]["ndcg10"].as_f64().unwrap();
    assert!(
        (n - 0.8).abs() < 1e-9,
        "expected the newer row (ndcg10=0.8), got {n}"
    );

    // Sanity: omitting `since` returns both, newest-first.
    let r2 = reqwest::Client::new()
        .get(format!("http://{addr}/admin/quality?limit=10"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    let body2: serde_json::Value = r2.json().await?;
    let all = body2["runs"].as_array().unwrap();
    assert_eq!(all.len(), 2);
    let first = all[0]["ndcg10"].as_f64().unwrap();
    let second = all[1]["ndcg10"].as_f64().unwrap();
    assert!(
        first >= second,
        "rows must be returned descending by ran_at (got ndcg10 first={first}, second={second})"
    );
    Ok(())
}
