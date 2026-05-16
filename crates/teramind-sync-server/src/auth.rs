//! Tower middleware: parse bearer + DPoP proof, attach AuthContext.

use crate::proof::{body_hash_hex, token_hash_hex, verify};
use crate::state::{AppState, AuthContext};
use crate::token::DeviceToken;
use axum::{
    body::Body, extract::{Request, State},
    http::{header, StatusCode}, middleware::Next, response::Response,
};
use time::OffsetDateTime;

pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let (mut parts, body) = request.into_parts();

    // 1. Parse Authorization.
    let bearer = parts.headers.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_string();
    let token = DeviceToken::parse(&bearer).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let token_hash = token.hash();
    let device = state.devices.get_active_by_token_hash(&token_hash).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // 2. Read X-Teramind-Proof + buffer body for hashing.
    let proof = parts.headers.get("x-teramind-proof")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?.to_string();
    let body_bytes = axum::body::to_bytes(body, state.cfg.ingest.max_request_body_bytes).await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let body_hash = body_hash_hex(&body_bytes);

    // 3. Verify proof.
    let url = matched_url(&parts);
    let method = parts.method.as_str().to_string();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims = verify(
        &proof, &device.public_key, &method, &url,
        &body_hash, &token_hash_hex(token.as_str()),
        now, state.cfg.auth.proof_replay_window_secs,
    ).map_err(|_| StatusCode::FORBIDDEN)?;

    // 4. Replay check.
    if !state.replay.check_and_insert(device.id, &claims.jti) {
        return Err(StatusCode::FORBIDDEN);
    }

    // 5. Fire-and-forget last-seen update.
    {
        let devices = state.devices.clone();
        let did = device.id;
        tokio::spawn(async move { let _ = devices.touch_last_seen(did).await; });
    }

    // 6. Attach AuthContext and rebuild the request.
    parts.extensions.insert(AuthContext {
        user_id: device.user_id, device_id: device.id,
    });
    let req = Request::from_parts(parts, Body::from(body_bytes));
    Ok(next.run(req).await)
}

/// Build the canonical absolute URL the client signed against.
fn matched_url(parts: &axum::http::request::Parts) -> String {
    let scheme = if parts.headers.get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok()) == Some("https")
        || parts.uri.scheme_str() == Some("https") { "https" } else { "http" };
    let host = parts.headers.get(header::HOST)
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    let path = parts.uri.path();
    format!("{scheme}://{host}{path}")
}
