use crate::admin_api::cookie::AdminSession;
use crate::state::AppState;
use axum::{
    extract::{Extension, State},
    Json,
};
use std::sync::OnceLock;
use std::time::Instant;

fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

pub async fn health(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> Json<serde_json::Value> {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.pool.pg())
        .await
        .is_ok();
    let backlog: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM skill_candidates WHERE status='pending'")
            .fetch_one(state.pool.pg())
            .await
            .unwrap_or(0);
    let uptime = process_start().elapsed().as_secs();
    Json(serde_json::json!({
        "db": if db_ok { "ok" } else { "down" },
        "broadcast_subscribers": state.bus.receiver_count(),
        "codifier_backlog": backlog,
        "team_sync": "n/a (server)",
        "quality_scheduler": {
            "enabled": state.cfg.quality.as_ref().map(|q| q.enabled).unwrap_or(false),
        },
        "ingest": {
            "queue_depth": 0,
            "accepted_24h": 0,
            "dropped_24h": 0,
        },
        "uptime_seconds": uptime,
    }))
}
