//! Pluggable transport for MCP and hook RPC.

use async_trait::async_trait;
use crate::proto::{Request, Response};

#[async_trait]
pub trait RpcTransport: Send + Sync {
    async fn request(&self, req: Request) -> Result<Response, RpcError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("connection failure: {0}")]
    Connect(String),
    #[error("server returned non-success: {0}")]
    Server(String),
    #[error("deserialize: {0}")]
    Decode(String),
    #[error("other: {0}")]
    Other(String),
}

impl RpcError {
    pub fn is_connect(&self) -> bool {
        matches!(self, RpcError::Connect(_))
    }
}
