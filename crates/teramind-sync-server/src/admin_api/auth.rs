//! Tower middleware: verify the admin session cookie.

use crate::admin_api::cookie::{decode, AdminSession};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use time::OffsetDateTime;

pub async fn admin_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(admin_cfg) = state.admin.as_ref() else {
        // Dashboard not configured — 404 to avoid signalling existence.
        return Err(StatusCode::NOT_FOUND);
    };
    let cookie_header = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = cookie_header
        .split(';')
        .map(|s| s.trim())
        .find_map(|kv| kv.strip_prefix("tmd_admin="))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let session: AdminSession = decode(
        token,
        &admin_cfg.admin_session_secret,
        OffsetDateTime::now_utc(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut req = request;
    req.extensions_mut().insert(session);
    Ok(next.run(req).await)
}
