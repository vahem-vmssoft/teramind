//! /admin/quality endpoints.

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use axum::{
    extract::{Extension, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use teramind_core::quality::QualityRunOutput;

#[derive(Deserialize)]
pub struct QualityQuery {
    pub since: Option<String>,
    pub baseline: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}
fn default_limit() -> i64 {
    60
}

pub async fn list(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<QualityQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let rows = state
        .quality
        .list_recent(q.baseline.as_deref(), q.limit)
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?;
    // Dashboard §5: ?since=<RFC3339 ts> filters to rows newer than ts.
    let rows: Vec<_> = if let Some(since_str) = q.since.as_deref() {
        match time::OffsetDateTime::parse(
            since_str,
            &time::format_description::well_known::Rfc3339,
        ) {
            Ok(since_ts) => rows.into_iter().filter(|r| r.ran_at > since_ts).collect(),
            Err(_) => {
                return Err(DashboardError::new(
                    StatusCode::BAD_REQUEST,
                    "validation_failed",
                    "since must be RFC3339",
                ))
            }
        }
    } else {
        rows
    };
    Ok(Json(serde_json::json!({
        "runs": rows.into_iter().map(|r| serde_json::json!({
            "id": r.id, "baseline_label": r.baseline_label, "model": r.model,
            "ndcg10": r.ndcg10, "mrr": r.mrr,
            "precision_5": r.precision_5, "precision_10": r.precision_10, "recall_10": r.recall_10,
            "p50_latency_ms": r.p50_latency_ms, "p95_latency_ms": r.p95_latency_ms,
            "query_count": r.query_count, "corpus_size": r.corpus_size,
            "ran_at": r.ran_at.to_string(),
            "source": r.source,
            "per_class": r.per_class,
        })).collect::<Vec<_>>()
    })))
}

pub async fn latest(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Query(q): Query<QualityQuery>,
) -> DashboardResult<Json<serde_json::Value>> {
    let baseline = q.baseline.clone().unwrap_or_else(|| "lexical".into());
    let row = state.quality.latest(&baseline).await.map_err(|e| {
        DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    Ok(Json(serde_json::json!({
        "run": row.map(|r| serde_json::json!({
            "id": r.id, "baseline_label": r.baseline_label, "model": r.model,
            "ndcg10": r.ndcg10, "mrr": r.mrr,
            "p50_latency_ms": r.p50_latency_ms, "p95_latency_ms": r.p95_latency_ms,
            "ran_at": r.ran_at.to_string(),
        }))
    })))
}

pub async fn upload(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Json(q): Json<QualityRunOutput>,
) -> DashboardResult<(StatusCode, Json<serde_json::Value>)> {
    if !q.ndcg10.is_finite()
        || !q.mrr.is_finite()
        || !(0.0..=1.0).contains(&q.ndcg10)
        || !(0.0..=1.0).contains(&q.mrr)
    {
        return Err(DashboardError::new(
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "metrics out of range",
        ));
    }
    let raw = serde_json::to_value(&q).unwrap_or_default();
    let id = state
        .quality
        .insert(
            &q.baseline_label,
            q.model.clone(),
            q.ndcg10,
            q.mrr,
            q.precision_5,
            q.precision_10,
            q.recall_10,
            q.p50_latency_ms,
            q.p95_latency_ms,
            q.query_count as i32,
            q.corpus_size as i32,
            q.per_class.clone(),
            raw,
            "manual",
        )
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

pub async fn config(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> Json<serde_json::Value> {
    use cron::Schedule;
    use std::str::FromStr;
    let qcfg = state.cfg.quality.as_ref();
    let cron_str = qcfg.and_then(|q| q.cron.clone());
    // next_run_at: only meaningful if quality is enabled AND cron parses.
    let next_run_at = match (qcfg.map(|q| q.enabled).unwrap_or(false), cron_str.as_ref()) {
        (true, Some(c)) => {
            let s = Schedule::from_str(&format!("0 {c}")).ok();
            s.and_then(|s| s.upcoming(chrono::Utc).next())
                .map(|t| t.to_rfc3339())
        }
        _ => None,
    };
    // last_run_at: most recent quality_runs row (across any baseline).
    let last_run_at = state
        .quality
        .list_recent(None, 1)
        .await
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .map(|r| r.ran_at.to_string());

    Json(serde_json::json!({
        "enabled": qcfg.map(|q| q.enabled).unwrap_or(false),
        "cron":    cron_str,
        "baselines": qcfg.map(|q| q.baselines.clone()).unwrap_or_default(),
        "last_run_at": last_run_at,
        "next_run_at": next_run_at,
    }))
}
