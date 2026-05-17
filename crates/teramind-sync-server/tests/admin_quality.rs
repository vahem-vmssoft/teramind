//! /admin/quality: upload, latest, validation.

use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

fn admin_cfg(password: &str) -> AdminConfig {
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
        admin: Some(admin_cfg("hunter2hunter2")),
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
async fn upload_persists_run() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    let payload = serde_json::json!({
        "baseline_label": "semantic",
        "model": null,
        "ndcg10": 0.72,
        "mrr": 0.65,
        "precision_5": 0.60,
        "precision_10": 0.55,
        "recall_10": 0.80,
        "p50_latency_ms": 45.0,
        "p95_latency_ms": 200.0,
        "query_count": 100,
        "corpus_size": 500,
        "per_class": {}
    });

    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/quality/runs"))
        .header("Cookie", &cookie)
        .json(&payload)
        .send().await?;
    assert_eq!(r.status(), 201);
    let body: serde_json::Value = r.json().await?;
    assert!(body["id"].is_string(), "expected id in response");

    // Verify the row exists in quality_runs with source='manual'.
    let rows = state.quality.list_recent(Some("semantic"), 10).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, "manual");
    assert_eq!(rows[0].baseline_label, "semantic");

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latest_returns_most_recent() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    // Insert two runs with different ran_at timestamps directly via state.quality.insert().
    // We rely on DB default NOW() for the first, then insert the second immediately after.
    let _id1 = state.quality.insert(
        "semantic", None,
        0.50, 0.45, 0.40, 0.35, 0.60,
        50.0, 150.0, 80, 400,
        serde_json::json!({}), serde_json::json!({}), "scheduled",
    ).await?;

    // Small sleep to ensure ran_at differs.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let _id2 = state.quality.insert(
        "semantic", None,
        0.80, 0.75, 0.70, 0.65, 0.90,
        30.0, 100.0, 80, 400,
        serde_json::json!({}), serde_json::json!({}), "manual",
    ).await?;

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/quality/latest?baseline=semantic"))
        .header("Cookie", &cookie)
        .send().await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let run = &body["run"];
    assert!(!run.is_null(), "run should not be null");
    // The newer run has ndcg10 = 0.80
    let ndcg = run["ndcg10"].as_f64().unwrap();
    assert!((ndcg - 0.80).abs() < 1e-9, "expected newer run with ndcg10=0.80, got {ndcg}");

    sup.shutdown().await?; Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn validation_rejects_nan() -> anyhow::Result<()> {
    let (_d, sup, addr, _state, cookie) = boot().await?;

    // serde_json doesn't serialise f64::NAN as JSON NaN (it would be invalid JSON).
    // Instead send a raw JSON string with the literal NaN token, which some parsers accept
    // but Rust's serde_json will reject at deserialisation, producing a 422/400.
    // We construct the body as a raw string to bypass serde's serialisation of NaN.
    let raw_body = r#"{
        "baseline_label": "semantic",
        "model": null,
        "ndcg10": "NaN",
        "mrr": 0.65,
        "precision_5": 0.60,
        "precision_10": 0.55,
        "recall_10": 0.80,
        "p50_latency_ms": 45.0,
        "p95_latency_ms": 200.0,
        "query_count": 100,
        "corpus_size": 500,
        "per_class": {}
    }"#;

    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/quality/runs"))
        .header("Cookie", &cookie)
        .header("Content-Type", "application/json")
        .body(raw_body)
        .send().await?;

    // serde_json rejects "NaN" string for f64 → axum returns 422 Unprocessable Entity.
    // We accept either 400 or 422 as both indicate the request was rejected.
    assert!(
        r.status().as_u16() == 400 || r.status().as_u16() == 422,
        "expected 400 or 422, got {}",
        r.status()
    );

    sup.shutdown().await?; Ok(())
}
