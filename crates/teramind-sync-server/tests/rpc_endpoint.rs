//! E2E: redeem an invite, POST to /v1/rpc, verify SearchResults round-trips.

use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::config::*;
use teramind_sync_server::invite::InviteCode;
use teramind_sync_server::proof::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::{Duration as TDur, OffsetDateTime};

#[allow(dead_code)]
struct Redeemed {
    user_id: String,
    device_id: String,
    token: String,
    signing_key: SigningKey,
}

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

async fn redeem(addr: SocketAddr, pool: &DbPool, email: &str) -> Redeemed {
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut rand::rng());
    invites
        .create(
            &code.hash(),
            email,
            None,
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await
        .unwrap();
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "dev1",
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    Redeemed {
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        token: body["device_token"].as_str().unwrap().into(),
        signing_key: sk,
    }
}

fn signed(addr: SocketAddr, path: &str, body: &[u8], r: &Redeemed) -> reqwest::RequestBuilder {
    let url = format!("http://{addr}{path}");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "POST".into(),
        htu: url.clone(),
        iat: now,
        jti: format!("jti-{}", uuid::Uuid::new_v4()),
        ath: token_hash_hex(&r.token),
        bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &r.signing_key);
    reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {}", r.token))
        .header("X-Teramind-Proof", proof)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_round_trips_through_rpc() -> anyhow::Result<()> {
    let (_d, sup, addr, pool) = boot().await?;
    let r = redeem(addr, &pool, "alice@acme.dev").await;
    let body = serde_json::to_vec(&teramind_ipc::proto::Request::Search(
        teramind_core::types::SearchRequest {
            query: "noop".into(),
            limit: 10,
        },
    ))?;
    let resp = signed(addr, "/v1/rpc", &body, &r).send().await?;
    assert_eq!(resp.status(), 200);
    let parsed: teramind_ipc::proto::Response = resp.json().await?;
    match parsed {
        teramind_ipc::proto::Response::SearchResults(_) => {}
        other => panic!("expected SearchResults, got {other:?}"),
    }
    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_without_auth_is_401() -> anyhow::Result<()> {
    let (_d, sup, addr, _pool) = boot().await?;
    let body = serde_json::to_vec(&teramind_ipc::proto::Request::Search(
        teramind_core::types::SearchRequest {
            query: "noop".into(),
            limit: 10,
        },
    ))?;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/rpc"))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await?;
    assert_eq!(resp.status(), 401);
    sup.shutdown().await?;
    Ok(())
}
