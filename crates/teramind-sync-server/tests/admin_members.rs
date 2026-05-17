//! /admin/members + /admin/devices + /admin/invites

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lists_members_with_device_counts() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    // Seed a user with two devices.
    let user = state
        .users
        .upsert_by_email("alice@example.com", None)
        .await?;
    state
        .devices
        .insert(user.id, "device-1", b"tok1hash_________", b"pubkey1")
        .await?;
    state
        .devices
        .insert(user.id, "device-2", b"tok2hash_________", b"pubkey2")
        .await?;

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/admin/members"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let users = body["users"].as_array().unwrap();
    let alice = users
        .iter()
        .find(|u| u["email"] == "alice@example.com")
        .unwrap();
    assert_eq!(alice["device_count"], 2);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creates_invite_and_returns_code_once() -> anyhow::Result<()> {
    let (_d, sup, addr, _state, cookie) = boot().await?;

    // POST /admin/invites → 201 + code starting with TM-
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/admin/invites"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "email": "bob@example.com",
            "expires_in_days": 7
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 201);
    let body: serde_json::Value = r.json().await?;
    let code = body["code"].as_str().unwrap();
    assert!(
        code.starts_with("TM-"),
        "code should start with TM-, got: {code}"
    );

    // GET /admin/invites → the invite is listed but WITHOUT the raw code
    let r2 = reqwest::Client::new()
        .get(format!("http://{addr}/admin/invites"))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r2.status(), 200);
    let list_body: serde_json::Value = r2.json().await?;
    let invites = list_body["invites"].as_array().unwrap();
    assert_eq!(invites.len(), 1);
    // The list endpoint should NOT expose the raw code
    assert!(
        invites[0].get("code").is_none(),
        "raw code must not appear in list response"
    );
    assert_eq!(invites[0]["invited_email"], "bob@example.com");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revokes_device_via_admin_endpoint() -> anyhow::Result<()> {
    let (_d, sup, addr, state, cookie) = boot().await?;

    // Seed user + device.
    let user = state
        .users
        .upsert_by_email("carol@example.com", None)
        .await?;
    let device = state
        .devices
        .insert(user.id, "carol-laptop", b"tok3hash_________", b"pubkey3")
        .await?;

    // Verify it shows up in the user's devices list before revoke.
    let pre = state.devices.list_for_user(user.id).await?;
    assert_eq!(pre.len(), 1);

    // POST /admin/devices/:id/revoke
    let r = reqwest::Client::new()
        .post(format!(
            "http://{addr}/admin/devices/{}/revoke",
            device.id.0
        ))
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert_eq!(body["revoked"], true);

    // Device should no longer be listed (revoked_at is set).
    let post = state.devices.list_for_user(user.id).await?;
    assert!(
        post.is_empty(),
        "device should be revoked and absent from active list"
    );

    sup.shutdown().await?;
    Ok(())
}
