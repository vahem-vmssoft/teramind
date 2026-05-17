//! `teramind feed [--follow]` — open the sync server's /v1/events WebSocket
//! and print one human-readable row per TeamEvent.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use futures_util::StreamExt;
use std::sync::Arc;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::team::TeamConfig;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio_tungstenite::tungstenite::{handshake::client::Request, Message};

pub async fn run(follow: bool, _backlog: u32) -> Result<()> {
    let cfg_dir = teramind_core::team::default_config_dir();
    let cfg = TeamConfig::load(&cfg_dir.join("team.toml"))
        .with_context(|| format!("load {}/team.toml — team mode required", cfg_dir.display()))?;
    let key = Arc::new(teramind_core::team::load_signing_key(
        &cfg_dir.join("team-key"),
    )?);

    let server_ws = cfg
        .server_url
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    let url_for_signing = format!("{}/v1/events", cfg.server_url);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = ProofClaims {
        htm: "GET".into(),
        htu: url_for_signing.clone(),
        iat: now,
        jti: format!("jti-{}", uuid::Uuid::new_v4()),
        ath: token_hash_hex(&cfg.device_token),
        bsh: body_hash_hex(b""),
    };
    let proof = sign(&claims, &key);
    let proof_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof.as_bytes());
    let ws_url = format!("{server_ws}/v1/events?proof={proof_b64}");

    let host = cfg
        .server_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();
    let req = Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {}", cfg.device_token))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())?;

    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .context("connect to /v1/events — is the server up + team.toml current?")?;
    let (_w, mut r) = ws.split();

    println!("{:<25} {:<15} details", "ts", "kind");
    while let Some(msg) = r.next().await {
        match msg? {
            Message::Text(txt) => {
                if let Ok(evt) = serde_json::from_str::<TeamEvent>(&txt) {
                    print_event(&evt);
                    if !follow {
                        return Ok(());
                    }
                }
                // Skip non-TeamEvent frames (e.g. hello).
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
    Err(anyhow!("server closed the connection"))
}

fn print_event(evt: &TeamEvent) {
    match evt {
        TeamEvent::SessionEnded { ts, cwd, .. } => {
            println!("{:<25} {:<15} {}", ts, "session_ended", cwd);
        }
        TeamEvent::WikiPageReady { ts, title, cwd, .. } => {
            println!("{:<25} {:<15} {} — {}", ts, "wiki_page_ready", cwd, title);
        }
        TeamEvent::SkillSaved { ts, name, .. } => {
            println!("{:<25} {:<15} {}", ts, "skill_saved", name);
        }
    }
}
