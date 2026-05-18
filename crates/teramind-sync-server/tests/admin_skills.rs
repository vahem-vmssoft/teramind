//! /admin/skills: list / show / delete.

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
async fn list_returns_codified_skills_only_when_filtered() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    // Seed one authored, one codified.
    let skill_repo = teramind_db::repos::SkillRepo::new(state.pool.clone());
    skill_repo
        .upsert_authored("authored-skill", "desc", "body authored")
        .await?;
    skill_repo
        .upsert_codified("codified-skill", "desc", "body codified", &[], &[])
        .await?;

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/skills?source=codified"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let skills = body["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0]["source"], "codified");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn show_returns_full_body() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    let skill_repo = teramind_db::repos::SkillRepo::new(state.pool.clone());
    let skill_id = skill_repo
        .upsert_authored("my-skill", "some desc", "the body text")
        .await?;

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/skills/{}", skill_id.0))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["name"], "my-skill");
    assert_eq!(body["body"], "the body text");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_removes_skill() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    let skill_repo = teramind_db::repos::SkillRepo::new(state.pool.clone());
    let skill_id = skill_repo
        .upsert_authored("to-delete", "desc", "body")
        .await?;

    let r = reqwest::Client::new()
        .delete(format!("http://{addr}/admin/skills/{}", skill_id.0))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["deleted"], true);

    // Subsequent GET returns 404.
    let r2 = reqwest::Client::new()
        .get(format!("http://{addr}/admin/skills/{}", skill_id.0))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r2.status(), 404);
    Ok(())
}
