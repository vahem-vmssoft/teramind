use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::repos::InviteRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::config::*;
use teramind_sync_server::invite::InviteCode;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::{Duration as TDur, OffsetDateTime};

async fn boot() -> anyhow::Result<(tempfile::TempDir, PgSupervisor, SocketAddr, DbPool)> {
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
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((dir, sup, addr, pool))
}

fn fresh_pk() -> Vec<u8> {
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    SigningKey::from_bytes(&seed)
        .verifying_key()
        .to_bytes()
        .to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy_path_issues_token() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let invites = InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x11u8; 16]);
    invites
        .create(
            &code.hash(),
            "alice@acme.dev",
            Some("Alice"),
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await?;
    let pk = fresh_pk();

    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(),
            "device_name": "alice-mac",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    assert!(body["device_token"]
        .as_str()
        .unwrap()
        .starts_with("tmd_v1_"));
    assert!(body["user_id"].is_string());
    assert!(body["device_id"].is_string());

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redeeming_twice_is_409() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let invites = InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x12u8; 16]);
    invites
        .create(
            &code.hash(),
            "alice@acme.dev",
            None,
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await?;
    let pk = fresh_pk();

    let send = || async {
        reqwest::Client::new()
            .post(format!("http://{addr}/v1/auth/redeem"))
            .json(&json!({
                "invite_code": code.as_str(),
                "device_name": "x",
                "device_public_key_b64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD, &pk),
            }))
            .send()
            .await
            .unwrap()
    };
    assert_eq!(send().await.status(), 200);
    assert_eq!(send().await.status(), 409);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_invite_is_410() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let invites = InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x13u8; 16]);
    invites
        .create(
            &code.hash(),
            "x@acme.dev",
            None,
            None,
            OffsetDateTime::now_utc() - TDur::seconds(1),
        )
        .await?;
    let pk = fresh_pk();
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "x",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 410);
    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_code_is_400() -> anyhow::Result<()> {
    let (_d, sup, addr, _pool) = boot().await?;
    let pk = fresh_pk();
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": "garbage", "device_name": "x",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 400);
    sup.shutdown().await?;
    Ok(())
}
