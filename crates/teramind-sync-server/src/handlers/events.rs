//! GET /v1/events WebSocket. Verifies the DPoP proof at upgrade time, then
//! streams TeamEvents from the AppState broadcast bus.

use crate::proof::verify;
use crate::state::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, StatusCode},
};
use base64::Engine;
use serde::Deserialize;
use teramind_core::team_event::TeamEvent;
use time::OffsetDateTime;
use tokio::sync::broadcast;

#[derive(Deserialize)]
pub struct EventsQuery {
    pub proof: String,
}

pub async fn events(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    // 1. Parse Authorization.
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_string();
    let token = crate::token::DeviceToken::parse(&bearer).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let device = state
        .devices
        .get_active_by_token_hash(&token.hash())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // 2. Decode proof from query string.
    let proof_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&q.proof)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let proof_str = std::str::from_utf8(&proof_bytes).map_err(|_| StatusCode::FORBIDDEN)?;

    // 3. Verify proof.
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let scheme = if headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https")
    {
        "https"
    } else {
        "http"
    };
    let url = format!("{scheme}://{host}/v1/events");
    let body_hash = teramind_core::dpop::body_hash_hex(b"");
    let token_hash = teramind_core::dpop::token_hash_hex(token.as_str());

    let claims = verify(
        proof_str,
        &device.public_key,
        "GET",
        &url,
        &body_hash,
        &token_hash,
        now,
        state.cfg.auth.proof_replay_window_secs,
    )
    .map_err(|e| {
        tracing::warn!(error = ?e, "events WS proof verify failed");
        StatusCode::FORBIDDEN
    })?;

    if !state.replay.check_and_insert(device.id, &claims.jti) {
        return Err(StatusCode::FORBIDDEN);
    }

    // 4. Upgrade + fan out.
    let rx = state.bus.subscribe();
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, rx)))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<TeamEvent>) {
    let hello = serde_json::json!({
        "type": "hello",
        "server_version": crate::VERSION,
    });
    if socket.send(Message::Text(hello.to_string())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(evt) => {
                        let json = match serde_json::to_string(&evt) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(json)).await.is_err() { return; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = socket.send(Message::Pong(payload)).await;
                    }
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
        }
    }
}
