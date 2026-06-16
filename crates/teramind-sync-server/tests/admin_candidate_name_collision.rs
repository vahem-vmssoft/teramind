//! Codifier §10 — a candidate whose proposed_name collides with an existing
//! authored skill that has DISJOINT source_session_ids must be rejected
//! synchronously, before promotion overwrites the authored skill.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approve_rejected_on_authored_name_collision() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    // 1. Pre-seed an authored skill (empty source_session_ids).
    let skill_repo = teramind_db::repos::SkillRepo::new(state.pool.clone());
    skill_repo
        .upsert_authored("rust-pr-prep", "author-written guide", "checklist body")
        .await?;

    // 2. Insert a pending candidate with the SAME name but a disjoint
    //    source_session_ids (a brand-new SessionId is guaranteed disjoint
    //    from the authored skill's empty set).
    use teramind_core::ids::SessionId;
    let obs_repo = teramind_db::repos::SkillObservationRepo::new(state.pool.clone());
    let sess = SessionId::new();
    obs_repo
        .upsert(
            "tool_chain",
            "rust-pr-prep",
            &[sess],
            serde_json::json!({"tool": "bash"}),
        )
        .await?;
    let obs = obs_repo
        .find_by_sig("tool_chain", "rust-pr-prep")
        .await?
        .expect("observation");
    let cand_repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let cand_id = cand_repo
        .insert(
            obs.id,
            "rust-pr-prep",
            "auto-generated",
            "candidate body",
            &["/workspace".into()],
            &[sess],
            "test-model",
            10,
            20,
        )
        .await?;

    // 3. Approve must be rejected with 4xx (conflict).
    let r = reqwest::Client::new()
        .post(format!(
            "http://{addr}/admin/candidates/{}/approve",
            cand_id.0
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    assert!(
        r.status().is_client_error(),
        "expected 4xx on name collision with authored skill, got {}",
        r.status()
    );
    assert_eq!(r.status(), 409);

    // The authored skill body must NOT have been overwritten.
    let body: (String, String) =
        sqlx::query_as("SELECT source, body FROM skills WHERE name='rust-pr-prep'")
            .fetch_one(state.pool.pg())
            .await?;
    assert_eq!(body.0, "authored");
    assert_eq!(body.1, "checklist body");
    Ok(())
}
