//! Dashboard §5.4 — POST .../reject moves a candidate to status=rejected;
//! a subsequent approve on the same id MUST NOT succeed (409 conflict).

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
    let state = AppState::new(pool, cfg);
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

async fn seed_candidate(state: &AppState, name: &str) -> anyhow::Result<uuid::Uuid> {
    use teramind_core::ids::SessionId;
    let obs_repo = teramind_db::repos::SkillObservationRepo::new(state.pool.clone());
    let sess = SessionId::new();
    obs_repo
        .upsert(
            "tool_chain",
            name,
            &[sess],
            serde_json::json!({"tool": "bash"}),
        )
        .await?;
    let obs = obs_repo
        .find_by_sig("tool_chain", name)
        .await?
        .expect("observation");
    let cand_repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let cand_id = cand_repo
        .insert(
            obs.id,
            name,
            "auto-generated",
            "body",
            &["/workspace".into()],
            &[sess],
            "test-model",
            10,
            20,
        )
        .await?;
    Ok(cand_id.0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reject_blocks_subsequent_approve() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;
    let cand_id = seed_candidate(&state, "reject-then-approve").await?;

    let r1 = reqwest::Client::new()
        .post(format!("http://{addr}/admin/candidates/{cand_id}/reject"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    assert_eq!(r1.status(), 200, "reject should succeed on pending");

    let r2 = reqwest::Client::new()
        .post(format!("http://{addr}/admin/candidates/{cand_id}/approve"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    assert_eq!(
        r2.status(),
        409,
        "approve after reject must be 409 conflict, got {}",
        r2.status()
    );

    // No codified skill row should have been created.
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM skills WHERE name='reject-then-approve' AND source='codified')",
    )
    .fetch_one(state.pool.pg())
    .await?;
    assert!(!exists, "rejected candidate must not be promoted to skill");
    Ok(())
}
