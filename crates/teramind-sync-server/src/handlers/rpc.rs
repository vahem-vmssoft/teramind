//! POST /v1/rpc — auth + DPoP protected. Dispatches Request -> Response via
//! the same dispatch fn the local daemon uses.

use crate::state::{AppState, AuthContext as ServerAuth};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use teramind_ipc::proto::{Request, Response};

pub async fn rpc(
    State(state): State<AppState>,
    Extension(auth): Extension<ServerAuth>,
    Json(req): Json<Request>,
) -> impl IntoResponse {
    let auth = teramindd::services::rpc_dispatch::AuthContext {
        user_id: auth.user_id.0,
        device_id: auth.device_id.0,
    };
    let deps = state.rpc_deps();
    let resp: Response = teramindd::services::rpc_dispatch::dispatch(&deps, req, Some(auth)).await;
    (StatusCode::OK, Json(resp))
}
