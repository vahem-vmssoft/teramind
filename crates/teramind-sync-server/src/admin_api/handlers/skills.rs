//! /admin/skills (list/show/delete)

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SkillsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    pub source: Option<String>,
    pub q: Option<String>,
}
fn default_limit() -> i64 {
    100
}

#[allow(clippy::type_complexity)]
pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<SkillsQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let src = q
        .source
        .as_deref()
        .filter(|s| matches!(*s, "authored" | "codified" | "imported"));
    let term = q.q.as_deref().unwrap_or("").to_string();
    let like = format!("%{}%", term);

    let rows: Vec<(uuid::Uuid, String, String, String, Vec<String>, Vec<uuid::Uuid>, time::OffsetDateTime, time::OffsetDateTime)> =
        sqlx::query_as(
            r#"SELECT id, name, description, source, applies_to_cwds, source_session_ids, created_at, updated_at
               FROM skills
               WHERE ($1::text IS NULL OR source = $1)
                 AND ($2::text = '' OR name ILIKE $3 OR description ILIKE $3)
               ORDER BY updated_at DESC
               LIMIT $4 OFFSET $5"#)
            .bind(src).bind(&term).bind(&like).bind(q.limit).bind(q.offset)
            .fetch_all(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;

    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM skills")
        .fetch_one(state.pool.pg())
        .await
        .unwrap_or((rows.len() as i64,));

    let skills = rows
        .into_iter()
        .map(|(id, name, desc, source, cwds, sids, created, updated)| {
            serde_json::json!({
                "id": id, "name": name, "description": desc, "source": source,
                "applies_to_cwds": cwds, "source_session_ids": sids,
                "created_at": created.to_string(), "updated_at": updated.to_string(),
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(
        serde_json::json!({ "skills": skills, "total": total }),
    ))
}

#[allow(clippy::type_complexity)]
pub async fn show(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id_or_name): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let row: Option<(uuid::Uuid, String, String, String, String, Vec<String>, Vec<uuid::Uuid>, time::OffsetDateTime, time::OffsetDateTime)> =
        sqlx::query_as(
            r#"SELECT id, name, description, body, source, applies_to_cwds, source_session_ids, created_at, updated_at
               FROM skills WHERE name = $1 OR id::text = $1 LIMIT 1"#)
            .bind(&id_or_name).fetch_optional(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let Some((id, name, description, body, source, cwds, sids, created, updated)) = row else {
        return Err(DashboardError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "no such skill",
        ));
    };
    Ok(Json(serde_json::json!({
        "id": id, "name": name, "description": description, "body": body, "source": source,
        "applies_to_cwds": cwds, "source_session_ids": sids,
        "created_at": created.to_string(), "updated_at": updated.to_string(),
    })))
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| {
        DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid")
    })?;
    let n = sqlx::query("DELETE FROM skills WHERE id = $1")
        .bind(id)
        .execute(state.pool.pg())
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "no such skill",
        ));
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}
