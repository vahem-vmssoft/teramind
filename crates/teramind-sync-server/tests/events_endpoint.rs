//! /v1/events: redeem, open WS with DPoP in ?proof=, send a TeamEvent via
//! the bus, assert the WS subscriber receives it.

use base64::Engine;
use ed25519_dalek::SigningKey;
use futures_util::StreamExt;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use std::time::Duration;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::team_event::TeamEvent;
use teramind_db::repos::InviteRepo;
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use time::{Duration as TDur, OffsetDateTime};
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_ended_event_streams_to_ws_subscriber() -> anyhow::Result<()> {
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
    let state = AppState::new(pool.clone(), cfg);
    let bus = state.bus.clone();
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Redeem.
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut rand::rng());
    invites
        .create(
            &code.hash(),
            "alice@acme.dev",
            None,
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await?;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(), "device_name": "dev",
            "device_public_key_b64": base64::engine::general_purpose::STANDARD.encode(&pk),
        }))
        .send()
        .await?;
    let body: serde_json::Value = r.json().await?;
    let token = body["device_token"].as_str().unwrap().to_string();

    // Build proof for GET /v1/events.
    let url_for_signing = format!("http://{addr}/v1/events");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "GET".into(),
        htu: url_for_signing.clone(),
        iat: now,
        jti: format!("jti-{}", Uuid::new_v4()),
        ath: token_hash_hex(&token),
        bsh: body_hash_hex(b""),
    };
    let proof = sign(&claims, &sk);
    let proof_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof.as_bytes());

    // Open WS.
    let ws_url = format!("ws://{addr}/v1/events?proof={proof_b64}");
    let req = Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())?;
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    let (_w, mut r) = ws.split();

    // Eat the hello frame.
    let _hello = tokio::time::timeout(Duration::from_secs(2), r.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("no hello frame"))??;

    // Publish a TeamEvent.
    let sid = Uuid::new_v4();
    let evt = TeamEvent::SessionEnded {
        session_id: sid,
        user_id: Uuid::new_v4(),
        cwd: "/x".into(),
        ts: OffsetDateTime::now_utc(),
    };
    let _ = bus.send(evt);

    // Receive it.
    let received = tokio::time::timeout(Duration::from_secs(2), r.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("no event frame"))??;
    match received {
        Message::Text(txt) => {
            let evt: TeamEvent = serde_json::from_str(&txt)?;
            match evt {
                TeamEvent::SessionEnded { session_id, .. } => assert_eq!(session_id, sid),
                other => panic!("unexpected: {other:?}"),
            }
        }
        other => panic!("expected text, got {other:?}"),
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_proof_is_rejected() -> anyhow::Result<()> {
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
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // No proof, no auth.
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/v1/events"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .send()
        .await?;
    assert!(
        r.status() == 400 || r.status() == 401,
        "expected 400/401, got {}",
        r.status()
    );
    Ok(())
}
