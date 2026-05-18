//! /admin/observations: list (kind filter, min_freq filter) + show.

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
async fn list_filters_by_kind_and_status() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    let obs_repo = teramind_db::repos::SkillObservationRepo::new(state.pool.clone());
    use teramind_core::ids::SessionId;
    let s1 = SessionId::new();
    let s2 = SessionId::new();
    obs_repo
        .upsert("tool_chain", "sig-tc", &[s1], serde_json::json!({}))
        .await?;
    obs_repo
        .upsert("problem_fix", "sig-fw", &[s2], serde_json::json!({}))
        .await?;

    // Filter by kind=tool_chain should return only 1.
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/observations?kind=tool_chain"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let obs = body["observations"].as_array().unwrap();
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0]["kind"], "tool_chain");    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_applies_min_freq_filter() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    let obs_repo = teramind_db::repos::SkillObservationRepo::new(state.pool.clone());
    use teramind_core::ids::SessionId;

    // Frequency 2: insert 2 distinct sessions.
    let s1 = SessionId::new();
    let s2 = SessionId::new();
    obs_repo
        .upsert("tool_chain", "low-freq-sig", &[s1], serde_json::json!({}))
        .await?;
    obs_repo
        .upsert("tool_chain", "low-freq-sig", &[s2], serde_json::json!({}))
        .await?;

    // Frequency 5: insert 5 distinct sessions.
    let sessions5: Vec<SessionId> = (0..5).map(|_| SessionId::new()).collect();
    for s in &sessions5 {
        obs_repo
            .upsert("tool_chain", "high-freq-sig", &[*s], serde_json::json!({}))
            .await?;
    }

    // min_freq=3 should only return the high-freq one.
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/observations?min_freq=3"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let obs = body["observations"].as_array().unwrap();
    assert_eq!(
        obs.len(),
        1,
        "only high-freq obs should pass min_freq=3 filter"
    );
    assert_eq!(obs[0]["signature"], "high-freq-sig");    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn show_returns_context_blob() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    let obs_repo = teramind_db::repos::SkillObservationRepo::new(state.pool.clone());
    use teramind_core::ids::SessionId;
    let s = SessionId::new();
    let ctx = serde_json::json!({ "tool": "bash", "command": "ls -la" });
    obs_repo
        .upsert("tool_chain", "show-sig", &[s], ctx.clone())
        .await?;
    let obs = obs_repo
        .find_by_sig("tool_chain", "show-sig")
        .await?
        .expect("must exist");

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/observations/{}", obs.id.0))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["context_blob"]["tool"], "bash");
    assert_eq!(body["context_blob"]["command"], "ls -la");    Ok(())
}
