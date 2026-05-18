//! Black-box tests for the auth middleware. Mounts an echo handler behind the
//! middleware on a random port and asserts 401 / 403 / 200 cases.

use axum::{routing::post, Extension, Json, Router};
use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::repos::{DeviceRepo, UserRepo};
use teramind_sync_server::auth::auth_middleware;
use teramind_sync_server::config::*;
use teramind_sync_server::proof::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_sync_server::state::{AppState, AuthContext};
use teramind_sync_server::token::DeviceToken;
use time::OffsetDateTime;

async fn echo(Extension(auth): Extension<AuthContext>) -> Json<serde_json::Value> {
    Json(json!({ "user": auth.user_id.0.to_string(), "device": auth.device_id.0.to_string() }))
}

fn fresh_signing_key() -> (SigningKey, Vec<u8>) {
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    (sk, pk)
}

async fn boot() -> anyhow::Result<(SocketAddr, AppState)> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let app = Router::new()
        .route("/v1/echo", post(echo))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((addr, state))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_authorization_is_401() -> anyhow::Result<()> {
    let (addr, _s) = boot().await?;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/echo"))
        .body("{}")
        .send()
        .await?;
    assert_eq!(r.status(), 401);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn valid_bearer_plus_proof_passes() -> anyhow::Result<()> {
    let (addr, state) = boot().await?;

    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (sk, pk) = fresh_signing_key();
    devices
        .insert(user.id, "alice-mac", &token.hash(), &pk)
        .await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    let body = br#"{"hello":1}"#;
    let url = format!("http://{addr}/v1/echo");
    let claims = ProofClaims {
        htm: "POST".into(),
        htu: url.clone(),
        iat: now,
        jti: "test-jti-1".into(),
        ath: token_hash_hex(token.as_str()),
        bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &sk);

    let r = reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .header("X-Teramind-Proof", proof)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .send()
        .await?;
    assert_eq!(r.status(), 200, "happy path must pass");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bearer_without_proof_is_403() -> anyhow::Result<()> {
    let (addr, state) = boot().await?;
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (_sk, pk) = fresh_signing_key();
    devices
        .insert(user.id, "alice-mac", &token.hash(), &pk)
        .await?;

    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/echo"))
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .body("{}")
        .send()
        .await?;
    assert_eq!(r.status(), 403);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proof_with_wrong_key_is_403() -> anyhow::Result<()> {
    let (addr, state) = boot().await?;
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (_registered_sk, registered_pk) = fresh_signing_key();
    let (attacker_sk, _) = fresh_signing_key();
    devices
        .insert(user.id, "alice-mac", &token.hash(), &registered_pk)
        .await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    let body = br#"{}"#;
    let url = format!("http://{addr}/v1/echo");
    let claims = ProofClaims {
        htm: "POST".into(),
        htu: url.clone(),
        iat: now,
        jti: "test-attack-jti".into(),
        ath: token_hash_hex(token.as_str()),
        bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &attacker_sk);

    let r = reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {}", token.as_str()))
        .header("X-Teramind-Proof", proof)
        .body(body.to_vec())
        .send()
        .await?;
    assert_eq!(
        r.status(),
        403,
        "stolen token without matching key must fail"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replayed_jti_is_403() -> anyhow::Result<()> {
    let (addr, state) = boot().await?;
    let users = UserRepo::new(state.pool.clone());
    let devices = DeviceRepo::new(state.pool.clone());
    let user = users.upsert_by_email("alice@acme.dev", None).await?;
    let token = DeviceToken::from_bytes([0xABu8; 32]);
    let (sk, pk) = fresh_signing_key();
    devices
        .insert(user.id, "alice-mac", &token.hash(), &pk)
        .await?;

    let now = OffsetDateTime::now_utc().unix_timestamp();
    let body = br#"{}"#;
    let url = format!("http://{addr}/v1/echo");
    let claims = ProofClaims {
        htm: "POST".into(),
        htu: url.clone(),
        iat: now,
        jti: "fixed-jti".into(),
        ath: token_hash_hex(token.as_str()),
        bsh: body_hash_hex(body),
    };
    let proof = sign(&claims, &sk);

    let client = reqwest::Client::new();
    let h = |p: &str| {
        client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token.as_str()))
            .header("X-Teramind-Proof", p)
            .body(body.to_vec())
    };
    let first = h(&proof).send().await?;
    assert_eq!(first.status(), 200);
    let second = h(&proof).send().await?;
    assert_eq!(second.status(), 403, "replayed jti must fail");
    Ok(())
}
