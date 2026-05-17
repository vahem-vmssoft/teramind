//! POST /v1/ingest — receive a batch of EventEnvelopes from a remote daemon
//! and dispatch each through teramindd's reusable route_with_deps().

use crate::state::{AppState, AuthContext};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::{Deserialize, Serialize};
use teramind_core::types::ingest_event::EventEnvelope;
use teramindd::{route_with_deps, IngestAuth};

#[derive(Deserialize)]
pub struct IngestBatch {
    pub events: Vec<EventEnvelope>,
}

#[derive(Serialize, Default)]
pub struct IngestSummary {
    pub accepted: u32,
    pub duplicates: u32,
    pub rejected: Vec<RejectedEvent>,
}

#[derive(Serialize)]
pub struct RejectedEvent {
    pub client_event_id: String,
    pub reason: String,
}

pub async fn ingest(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(batch): Json<IngestBatch>,
) -> impl IntoResponse {
    if batch.events.len() > state.cfg.ingest.max_batch_size {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(IngestSummary::default()),
        )
            .into_response();
    }
    let rd = state.route_deps();
    let ia = IngestAuth {
        user_id: auth.user_id.0,
        device_id: auth.device_id.0,
    };

    let mut summary = IngestSummary::default();
    for env in batch.events {
        let cid = env.client_event_id.0.to_string();
        match route_with_deps(&rd, env, Some(ia)).await {
            Ok(()) => summary.accepted += 1,
            Err(e) => {
                let s = e.to_string();
                if s.contains("duplicate key") || s.contains("unique constraint") {
                    summary.duplicates += 1;
                } else {
                    summary.rejected.push(RejectedEvent {
                        client_event_id: cid,
                        reason: s,
                    });
                }
            }
        }
    }
    (StatusCode::OK, Json(summary)).into_response()
}
