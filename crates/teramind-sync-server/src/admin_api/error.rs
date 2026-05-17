//! Stable error JSON shape for /admin/*.

use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DashboardError {
    pub error: ErrorBody,
    #[serde(skip)]
    pub status: StatusCode,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl DashboardError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self { status, error: ErrorBody { code: code.into(), message: message.into(), details: None } }
    }
}

impl IntoResponse for DashboardError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.error }));
        (self.status, body).into_response()
    }
}

pub type DashboardResult<T> = Result<T, DashboardError>;
