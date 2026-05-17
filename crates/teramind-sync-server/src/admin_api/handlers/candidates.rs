//! /admin/candidates list / show / approve / reject / PATCH.

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
pub struct CandidatesQuery {
    #[serde(default = "default_limit")] pub limit: i64,
    #[serde(default)] pub offset: i64,
    pub status: Option<String>,
}
fn default_limit() -> i64 { 100 }

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<CandidatesQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let rows = repo.list_filter(q.status.as_deref(), q.limit).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(Json(serde_json::json!({
        "candidates": rows.iter().map(|c| serde_json::json!({
            "id": c.id.0, "observation_id": c.observation_id.0,
            "name": c.name, "description": c.description, "body": c.body,
            "applies_to_cwds": c.applies_to_cwds,
            "source_session_ids": c.source_session_ids,
            "model": c.model,
            "input_tokens": c.input_tokens, "output_tokens": c.output_tokens,
            "generated_at": c.generated_at.to_string(),
            "status": c.status,
            "reviewer": c.reviewer,
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
    let row: Option<(uuid::Uuid, uuid::Uuid, String, String, String, Vec<String>, Vec<uuid::Uuid>, String, i32, i32, time::OffsetDateTime, String, Option<String>, Option<time::OffsetDateTime>)> =
        sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates WHERE id = $1"#)
            .bind(id).fetch_optional(state.pool.pg()).await
            .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let Some(r) = row else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "no such candidate"));
    };
    Ok(Json(serde_json::json!({
        "id": r.0, "observation_id": r.1, "name": r.2, "description": r.3, "body": r.4,
        "applies_to_cwds": r.5, "source_session_ids": r.6, "model": r.7,
        "input_tokens": r.8, "output_tokens": r.9, "generated_at": r.10.to_string(),
        "status": r.11, "reviewer": r.12, "reviewed_at": r.13.map(|t| t.to_string()),
    })))
}

#[derive(Deserialize)]
pub struct ApproveBody { pub reviewer: Option<String> }

pub async fn approve(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let reviewer = body.reviewer.unwrap_or_else(|| "admin".into());
    let n = sqlx::query(
        "UPDATE skill_candidates SET status='approved', reviewer=$2, reviewed_at=now()
         WHERE id=$1 AND status='pending'")
        .bind(id).bind(&reviewer).execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::CONFLICT, "conflict", "candidate not pending"));
    }
    // Synchronous promotion so the UI sees the live skill immediately.
    let cand_repo = teramind_db::repos::SkillCandidateRepo::new(state.pool.clone());
    let skill_repo = teramind_db::repos::SkillRepo::new(state.pool.clone());
    let _ = teramindd::services::codify::promote::promote_approved_batch(
        &state.pool, &cand_repo, &skill_repo, 10,
    ).await;
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT s.id FROM skill_candidates c JOIN skills s ON s.name = c.name
         WHERE c.id = $1")
        .bind(id).fetch_optional(state.pool.pg()).await.ok().flatten();
    Ok(Json(serde_json::json!({ "skill_id": row.map(|(id,)| id) })))
}

#[derive(Deserialize)]
pub struct RejectBody { pub reviewer: Option<String>, pub reason: Option<String> }

pub async fn reject(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
    Json(body): Json<RejectBody>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let reviewer = body.reviewer.unwrap_or_else(|| "admin".into());
    let n = sqlx::query(
        "UPDATE skill_candidates SET status='rejected', reviewer=$2, reviewed_at=now()
         WHERE id=$1 AND status='pending'")
        .bind(id).bind(&reviewer).execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::CONFLICT, "conflict", "candidate not pending"));
    }
    let _ = body.reason;  // reserved; not persisted in v1
    Ok(Json(serde_json::json!({ "rejected": true })))
}

#[derive(Deserialize)]
pub struct PatchBody {
    pub description: Option<String>,
    pub body: Option<String>,
    pub applies_to_cwds: Option<Vec<String>>,
}

pub async fn patch(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(id): Path<String>,
    Json(p): Json<PatchBody>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid"))?;
    let n = sqlx::query(
        r#"UPDATE skill_candidates
           SET description = COALESCE($2, description),
               body        = COALESCE($3, body),
               applies_to_cwds = COALESCE($4, applies_to_cwds)
           WHERE id = $1 AND status = 'pending'"#)
        .bind(id).bind(p.description).bind(p.body).bind(p.applies_to_cwds)
        .execute(state.pool.pg()).await
        .map_err(|e| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .rows_affected();
    if n == 0 {
        return Err(DashboardError::new(StatusCode::CONFLICT, "conflict", "candidate not pending"));
    }
    Ok(Json(serde_json::json!({ "updated": true })))
}
