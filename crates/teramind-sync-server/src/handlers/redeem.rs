//! POST /v1/auth/redeem — exchange an invite code + a device public key for
//! a long-lived bearer token. Atomic transaction: upsert user, insert device,
//! mark invite redeemed.

use crate::invite::InviteCode;
use crate::state::AppState;
use crate::token::DeviceToken;
use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use teramind_db::repos::Device;

#[derive(Deserialize)]
pub struct RedeemRequest {
    pub invite_code: String,
    pub device_name: String,
    pub device_public_key_b64: String,
}

#[derive(Serialize)]
pub struct RedeemResponse {
    pub user_id: String,
    pub device_id: String,
    pub device_token: String,
    pub device_name: String,
}

pub async fn redeem(
    State(state): State<AppState>,
    Json(req): Json<RedeemRequest>,
) -> Result<(StatusCode, Json<RedeemResponse>), (StatusCode, String)> {
    let code = InviteCode::parse(&req.invite_code)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad invite: {e}")))?;
    let pk = B64.decode(&req.device_public_key_b64).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "device_public_key_b64 must be base64".into(),
        )
    })?;
    if pk.len() != 32 {
        return Err((
            StatusCode::BAD_REQUEST,
            "device_public_key must be 32 bytes".into(),
        ));
    }
    if req.device_name.is_empty() || req.device_name.len() > 200 {
        return Err((StatusCode::BAD_REQUEST, "device_name length 1..=200".into()));
    }

    let code_hash = code.hash();
    let invite = match state
        .invites
        .find_redeemable(&code_hash)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?
    {
        Some(inv) => inv,
        None => {
            // Distinguish "already redeemed" (409) from "missing or expired" (410).
            let existing = state
                .invites
                .find_by_hash(&code_hash)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;
            match existing {
                Some(inv) if inv.redeemed_at.is_some() => {
                    return Err((StatusCode::CONFLICT, "invite already redeemed".into()));
                }
                _ => return Err((StatusCode::GONE, "invite not found or expired".into())),
            }
        }
    };

    let user = state
        .users
        .upsert_by_email(&invite.invited_email, invite.display_name.as_deref())
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;

    let token = DeviceToken::generate(&mut OsRng);
    let device: Device = state
        .devices
        .insert(user.id, &req.device_name, &token.hash(), &pk)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;

    let n = state
        .invites
        .mark_redeemed(&code_hash, device.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db".into()))?;
    if n == 0 {
        let _ = state.devices.revoke(device.id).await;
        return Err((StatusCode::CONFLICT, "invite already redeemed".into()));
    }

    Ok((
        StatusCode::OK,
        Json(RedeemResponse {
            user_id: user.id.0.to_string(),
            device_id: device.id.0.to_string(),
            device_token: token.as_str().to_string(),
            device_name: device.name,
        }),
    ))
}
