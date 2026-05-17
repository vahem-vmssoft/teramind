//! /admin/login, /admin/logout, /admin/me, /admin/version

use crate::admin_api::cookie::{encode, random_jti, AdminSession};
use crate::admin_api::error::{DashboardError, DashboardResult};
use crate::state::AppState;
use argon2::{password_hash::PasswordHash, Argon2, PasswordVerifier};
use axum::{
    extract::{ConnectInfo, Extension, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use time::{Duration, OffsetDateTime};

#[derive(Deserialize)]
pub struct LoginRequest { pub password: String }

#[derive(Serialize)]
pub struct LoginResponse {
    pub logged_in: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> DashboardResult<impl IntoResponse> {
    let Some(admin_cfg) = state.admin.as_ref() else {
        return Err(DashboardError::new(StatusCode::NOT_FOUND, "not_found", "dashboard not configured"));
    };
    let ip = addr.ip();
    if let Err(_remain) = state.login_throttle.check(ip) {
        return Err(DashboardError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many failed attempts"));
    }

    let parsed = PasswordHash::new(&admin_cfg.admin_password_hash)
        .map_err(|_| DashboardError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "bad password hash in config"))?;
    if Argon2::default().verify_password(req.password.as_bytes(), &parsed).is_err() {
        state.login_throttle.record_failure(ip);
        return Err(DashboardError::new(StatusCode::UNAUTHORIZED, "invalid_password", "bad password"));
    }
    state.login_throttle.record_success(ip);

    let session = AdminSession {
        jti: random_jti(),
        expires_at: OffsetDateTime::now_utc() + Duration::hours(admin_cfg.admin_session_ttl_hours as i64),
    };
    let token = encode(&session, &admin_cfg.admin_session_secret);
    let max_age = admin_cfg.admin_session_ttl_hours * 3600;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        format!(
            "tmd_admin={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={max_age}"
        ).parse().unwrap(),
    );
    Ok((headers, Json(LoginResponse { logged_in: true, expires_at: session.expires_at })))
}

pub async fn logout() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        "tmd_admin=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0".parse().unwrap(),
    );
    (headers, Json(serde_json::json!({ "logged_out": true })))
}

#[derive(Serialize)]
pub struct MeResponse {
    pub admin: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

pub async fn me(Extension(session): Extension<AdminSession>) -> Json<MeResponse> {
    Json(MeResponse { admin: true, expires_at: session.expires_at })
}

pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "version": crate::VERSION }))
}
