//! /admin/activity (HTTP GET) + /admin/events (WebSocket)

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use teramind_core::ids::UserId;
use teramind_core::team_event::TeamEvent;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ActivityQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub before: Option<String>,
    pub kind: Option<String>,
    pub user_id: Option<String>,
}
fn default_limit() -> i64 {
    100
}

pub async fn activity(
    State(state): State<AppState>,
    Extension(_session): Extension<AdminSession>,
    Query(q): Query<ActivityQuery>,
) -> DashboardResult<impl IntoResponse> {
    let before = q.before.as_deref().and_then(|s| {
        time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
    });
    let user_id = q
        .user_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(UserId);
    let rows = state
        .event_log
        .list_recent(q.kind.as_deref(), before, user_id, q.limit)
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?;
    let next_before = rows.last().map(|r| r.ts.to_string());
    Ok(Json(serde_json::json!({
        "events": rows.iter().map(|r| serde_json::json!({
            "id": r.id, "kind": r.kind, "user_id": r.user_id.map(|u| u.0),
            "cwd": r.cwd, "payload": r.payload, "ts": r.ts.to_string(),
        })).collect::<Vec<_>>(),
        "next_before": next_before,
    })))
}

pub async fn events_ws(
    State(state): State<AppState>,
    Extension(_session): Extension<AdminSession>,
    ws: WebSocketUpgrade,
) -> Response {
    let rx = state.bus.subscribe();
    ws.on_upgrade(move |socket| handle_socket(socket, rx))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<TeamEvent>) {
    let hello = serde_json::json!({ "type": "hello" });
    if socket.send(Message::Text(hello.to_string().into())).await.is_err() {
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
                        if socket.send(Message::Text(json.into())).await.is_err() { return; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            inc = socket.recv() => {
                match inc {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(Message::Ping(p))) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
        }
    }
}
