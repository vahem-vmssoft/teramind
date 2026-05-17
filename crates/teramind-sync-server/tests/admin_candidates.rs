//! /admin/candidates: approve / second-approve / patch.

use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
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

async fn boot() -> anyhow::Result<(
    tempfile::TempDir,
    PgSupervisor,
    SocketAddr,
    AppState,
    String,
)> {
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
    Ok((dir, sup, addr, state, cookie))
}

/// Seed an observation + candidate, return the candidate UUID.
async fn seed_candidate(state: &AppState, name: &str) -> anyhow::Result<uuid::Uuid> {
    use teramind_core::ids::SessionId;
    let obs_repo = teramind_db::repos::SkillObservationRepo::new(state.pool.clone());
    let sess = SessionId::new();
    obs_repo
        .upsert(
            "tool_chain",
            name,
            &[sess],
            serde_json::json!({"tool":"bash"}),
        )
        .await?;
    let obs = obs_repo
        .find_by_sig("tool_chain", name)
        .await?
        .expect("obs must exist");

    let cand_repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let cand_id = cand_repo
        .insert(
            obs.id,
            name,
            "auto-generated description",
            "the skill body",
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
async fn approve_synchronously_promotes() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;
    let cand_id = seed_candidate(&state, "promote-skill").await?;

    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/candidates/{cand_id}/approve"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;

    // skill_id must be present and non-null.
    let skill_id = body["skill_id"]
        .as_str()
        .expect("skill_id should be a uuid string");
    assert!(!skill_id.is_empty());

    // Verify the skill row exists in DB directly.
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM skills WHERE name='promote-skill' AND source='codified')",
    )
    .fetch_one(state.pool.pg())
    .await?;
    assert!(exists, "skill row must exist after approve");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_approve_is_409() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;
    let cand_id = seed_candidate(&state, "double-approve-skill").await?;

    // First approve: 200.
    let r1 = reqwest::Client::new()
        .post(format!("http://{addr}/admin/candidates/{cand_id}/approve"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    assert_eq!(r1.status(), 200);

    // Second approve on same candidate: 409.
    let r2 = reqwest::Client::new()
        .post(format!("http://{addr}/admin/candidates/{cand_id}/approve"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    assert_eq!(r2.status(), 409);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn patch_updates_body_keeps_pending() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;
    let cand_id = seed_candidate(&state, "patchable-skill").await?;

    let r = reqwest::Client::new()
        .patch(format!("http://{addr}/admin/candidates/{cand_id}"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "body": "updated body content" }))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["updated"], true);

    // GET should return the new body and status=pending.
    let r2 = reqwest::Client::new()
        .get(format!("http://{addr}/admin/candidates/{cand_id}"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r2.status(), 200);
    let got: serde_json::Value = r2.json().await?;
    assert_eq!(got["body"], "updated body content");
    assert_eq!(got["status"], "pending");

    sup.shutdown().await?;
    Ok(())
}
