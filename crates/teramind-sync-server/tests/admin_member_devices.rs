//! Dashboard §5 — GET /admin/members/<user_id>/devices returns the user's
//! devices as a JSON array, each entry carrying created_at, last_seen_at
//! (nullable), and revoked_at (nullable).

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
async fn lists_user_devices_with_required_fields() -> anyhow::Result<()> {
    let (addr, state, cookie) = boot().await?;

    // Seed user + device directly (faster than redeem flow, exercises same repo).
    let user = state
        .users
        .upsert_by_email("dee@acme.dev", Some("Dee"))
        .await?;
    let device = state
        .devices
        .insert(user.id, "dee-laptop", b"tokhash__________", b"pubkey1")
        .await?;

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/members/{}/devices", user.id.0))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let arr = body
        .as_array()
        .expect("response must be a JSON array of devices");
    assert!(!arr.is_empty(), "must contain the seeded device");
    let entry = arr
        .iter()
        .find(|e| e["id"].as_str() == Some(&device.id.0.to_string()))
        .expect("seeded device must be in the list");
    // All three keys must be present (even if value is null for last_seen / revoked).
    for key in &["created_at", "last_seen_at", "revoked_at"] {
        assert!(
            entry.get(*key).is_some(),
            "device entry must include `{key}` key (got: {entry})"
        );
    }
    // created_at must NOT be null (devices.created_at has NOT NULL DEFAULT now()).
    assert!(
        !entry["created_at"].is_null(),
        "created_at must be set for an inserted device"
    );
    // The other two are nullable in steady state.
    Ok(())
}
