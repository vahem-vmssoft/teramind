use crate::IpcError;
use std::path::PathBuf;
use tokio::net::{UnixListener, UnixStream};

pub fn default_socket_path() -> PathBuf {
    PathBuf::from(std::env::var("TERAMIND_SOCKET").unwrap_or_else(|_| "/tmp/teramind.sock".into()))
}

pub async fn connect(path: &std::path::Path) -> Result<UnixStream, IpcError> {
    Ok(UnixStream::connect(path).await?)
}

pub fn listen(path: &std::path::Path) -> Result<UnixListener, IpcError> {
    if path.exists() { let _ = std::fs::remove_file(path); }
    Ok(UnixListener::bind(path)?)
}
