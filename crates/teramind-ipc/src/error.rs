use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("daemon busy")]
    Busy,
    #[error("daemon unreachable")]
    Unreachable,
    #[error("protocol error: {0}")]
    Protocol(String),
}
