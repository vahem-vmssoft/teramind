use crate::IpcError;
use std::path::PathBuf;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

pub fn default_socket_path() -> PathBuf {
    PathBuf::from(r"\\.\pipe\teramind")
}

pub async fn connect(path: &std::path::Path) -> Result<NamedPipeClient, IpcError> {
    let s = path.to_string_lossy();
    Ok(ClientOptions::new().open(s.as_ref())?)
}

pub fn listen(path: &std::path::Path) -> Result<NamedPipeServer, IpcError> {
    let s = path.to_string_lossy();
    Ok(ServerOptions::new().first_pipe_instance(true).create(s.as_ref())?)
}
