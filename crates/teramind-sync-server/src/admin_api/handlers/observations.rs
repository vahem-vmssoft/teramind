//! /admin/observations list + show.

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{extract::{Extension, Path, Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use teramind_db::repos::SkillObservationRepo;

#[derive(Deserialize)]
pub struct ObsQuery {
    pub kind: Option<String>,
    pub status: Option<String>,
    #[serde(default)] pub min_freq: i32,
    #[serde(default = "default_limit")] pub limit: i64,
}
fn default_limit() -> i64 { 100 }

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<ObsQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let repo = SkillObservationRepo::new(state.pool.clone());
    let rows = repo.list_recent(q.kind.as_deref(), q.status.as_deref(), q.limit).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({
        "observations": rows.iter().filter(|o| o.frequency >= q.min_freq).map(|o| serde_json::json!({
            "id": o.id.0, "kind": o.kind, "signature": o.signature,
            "frequency": o.frequency, "status": o.status,
            "last_seen_at": o.last_seen_at.to_string(),
        })).collect::<Vec<_>>(),
        "total": rows.len(),
    })))
}

#[allow(clippy::type_complexity)]
pub async fn show(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let row: Option<(uuid::Uuid, String, String, Vec<uuid::Uuid>, i32, serde_json::Value, time::OffsetDateTime, time::OffsetDateTime, String)> =
        sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations WHERE id = $1"#)
            .bind(id).fetch_optional(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let Some(r) = row else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "no such observation"));
    };
    Ok(Json(serde_json::json!({
        "id": r.0, "kind": r.1, "signature": r.2, "session_ids": r.3,
        "frequency": r.4, "context_blob": r.5,
        "first_seen_at": r.6.to_string(), "last_seen_at": r.7.to_string(),
        "status": r.8,
    })))
}
