use crate::state::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    db: &'static str,
}

#[derive(Serialize)]
struct Version {
    version: &'static str,
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.pool.pg())
        .await
        .is_ok();
    let body = Health {
        status: if ok { "ok" } else { "degraded" },
        db: if ok { "ok" } else { "down" },
    };
    if ok {
        (StatusCode::OK, Json(body))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body))
    }
}

pub async fn version() -> impl IntoResponse {
    Json(Version {
        version: crate::VERSION,
    })
}
