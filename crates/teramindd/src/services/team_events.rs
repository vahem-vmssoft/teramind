//! Subscribes to the server's /v1/events WebSocket and republishes received
//! TeamEvents on a local broadcast bus. Reconnects with exponential backoff.

use anyhow::{Context, Result};
use base64::Engine;
use ed25519_dalek::SigningKey;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::team::TeamConfig;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};
use tracing::{info, warn};
use futures_util::StreamExt;

pub struct TeamEventsDeps {
    pub team_cfg: Arc<TeamConfig>,
    pub signing_key: Arc<SigningKey>,
    pub local_bus: broadcast::Sender<TeamEvent>,
}

pub struct TeamEvents {
    _handle: tokio::task::JoinHandle<()>,
}

impl TeamEvents {
    pub fn spawn(deps: TeamEventsDeps) -> Self {
        let handle = tokio::spawn(async move { run_loop(deps).await; });
        Self { _handle: handle }
    }
}

async fn run_loop(deps: TeamEventsDeps) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect_and_pump(&deps).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => warn!(error = %e, ?backoff, "team_events connect failed; reconnecting"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn connect_and_pump(deps: &TeamEventsDeps) -> Result<()> {
    let server_ws = deps.team_cfg.server_url.replace("http://", "ws://").replace("https://", "wss://");
    let url_for_signing = format!("{}/v1/events", deps.team_cfg.server_url);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "GET".into(), htu: url_for_signing.clone(), iat: now,
        jti: format!("jti-{}", uuid::Uuid::new_v4()),
        ath: token_hash_hex(&deps.team_cfg.device_token),
        bsh: body_hash_hex(b""),
    };
    let proof = sign(&claims, &deps.signing_key);
    let proof_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof.as_bytes());

    let ws_url = format!("{server_ws}/v1/events?proof={proof_b64}");
    let host = deps.team_cfg.server_url
        .trim_start_matches("http://").trim_start_matches("https://")
        .to_string();
    let req = Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {}", deps.team_cfg.device_token))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(()).context("build ws request")?;

    let (ws, _resp) = tokio_tungstenite::connect_async(req).await
        .context("ws connect")?;
    info!(%ws_url, "team_events connected");
    let (_w, mut r) = ws.split();
    while let Some(msg) = r.next().await {
        let msg = msg.context("ws recv")?;
        if let Message::Text(text) = msg {
            if let Ok(evt) = serde_json::from_str::<TeamEvent>(&text) {
                let _ = deps.local_bus.send(evt);
            }
        }
    }
    Ok(())
}
