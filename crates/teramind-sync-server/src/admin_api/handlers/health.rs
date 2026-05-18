use crate::admin_api::cookie::AdminSession;
use crate::state::AppState;
use axum::{
    extract::{Extension, State},
    Json,
};

pub async fn health(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> Json<serde_json::Value> {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.pool.pg())
        .await
        .is_ok();
    Json(serde_json::json!({
        "db": if db_ok { "ok" } else { "down" },
        "broadcast_subscribers": state.bus.receiver_count(),
        "quality_scheduler": {
            "enabled": state.cfg.quality.as_ref().map(|q| q.enabled).unwrap_or(false),
        },
    }))
}
