//! /admin/members + /admin/devices + /admin/invites

use crate::admin_api::cookie::AdminSession;
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::invite::InviteCode;
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use teramind_core::ids::{DeviceId, InviteId, UserId};
use time::{Duration, OffsetDateTime};

type UserRow = (
    uuid::Uuid,
    String,
    Option<String>,
    time::OffsetDateTime,
    Option<time::OffsetDateTime>,
);

pub async fn members(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> DashboardResult<Json<serde_json::Value>> {
    let rows: Vec<UserRow> = sqlx::query_as(
        r#"SELECT id, email, display_name, created_at, revoked_at FROM users ORDER BY email"#,
    )
    .fetch_all(state.pool.pg())
    .await
    .map_err(|e| {
        DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    let mut out = vec![];
    for (uid, email, name, created, revoked) in rows {
        let counts: Vec<(i64, Option<time::OffsetDateTime>)> = sqlx::query_as(
            r#"SELECT count(*), max(last_seen_at)::timestamptz FROM devices
               WHERE user_id = $1 AND revoked_at IS NULL"#,
        )
        .bind(uid)
        .fetch_all(state.pool.pg())
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?;
        let (device_count, last_seen) = counts.first().cloned().unwrap_or((0, None));
        out.push(serde_json::json!({
            "id": uid, "email": email, "display_name": name,
            "created_at": created.to_string(),
            "revoked_at": revoked.map(|t| t.to_string()),
            "device_count": device_count,
            "last_seen_at": last_seen.map(|t| t.to_string()),
        }));
    }
    Ok(Json(serde_json::json!({ "users": out })))
}

pub async fn revoke_user(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(user_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = UserId(uuid::Uuid::parse_str(&user_id).map_err(|_| {
        DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid")
    })?);
    state.users.revoke(id).await.map_err(|e| {
        DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

pub async fn user_devices(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(user_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = UserId(uuid::Uuid::parse_str(&user_id).map_err(|_| {
        DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid")
    })?);
    // Dashboard §5: include revoked devices too so admins can audit history.
    let devices = state
        .devices
        .list_for_user_including_revoked(id)
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?;
    let json = devices
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id.0,
                "name": d.name,
                "created_at": d.created_at.map(|t| t.to_string()),
                "last_seen_at": d.last_seen_at.map(|t| t.to_string()),
                "revoked_at": d.revoked_at.map(|t| t.to_string()),
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::Value::Array(json)))
}

pub async fn revoke_device(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(device_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = DeviceId(uuid::Uuid::parse_str(&device_id).map_err(|_| {
        DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid")
    })?);
    state.devices.revoke(id).await.map_err(|e| {
        DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

pub async fn list_invites(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
) -> DashboardResult<Json<serde_json::Value>> {
    let invites = state.invites.list_outstanding().await.map_err(|e| {
        DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    let json = invites
        .into_iter()
        .map(|i| {
            serde_json::json!({
                "id": i.id.0, "invited_email": i.invited_email, "display_name": i.display_name,
                "created_by": i.created_by, "created_at": i.created_at.to_string(),
                "expires_at": i.expires_at.to_string(),
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "invites": json })))
}

#[derive(Deserialize)]
pub struct NewInvite {
    pub email: String,
    pub display_name: Option<String>,
    pub expires_in_days: Option<i64>,
}

pub async fn create_invite(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Json(body): Json<NewInvite>,
) -> DashboardResult<(StatusCode, Json<serde_json::Value>)> {
    let code = InviteCode::generate(&mut rand::rng());
    let days = body
        .expires_in_days
        .unwrap_or(state.cfg.auth.invite_default_expires_days);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(days);
    let id = state
        .invites
        .create(
            &code.hash(),
            &body.email,
            body.display_name.as_deref(),
            Some("admin"),
            expires_at,
        )
        .await
        .map_err(|e| {
            DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
        })?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "invite_id": id.0,
            "code": code.as_str(),
            "expires_at": expires_at.to_string(),
        })),
    ))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    Extension(_): Extension<AdminSession>,
    Path(invite_id): Path<String>,
) -> DashboardResult<Json<serde_json::Value>> {
    let id = InviteId(uuid::Uuid::parse_str(&invite_id).map_err(|_| {
        DashboardError::new(StatusCode::BAD_REQUEST, "validation_failed", "bad uuid")
    })?);
    state.invites.revoke(id).await.map_err(|e| {
        DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}
