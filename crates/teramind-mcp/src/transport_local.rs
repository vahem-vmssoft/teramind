//! UDS / named-pipe transport — wraps the existing IpcClient.

use async_trait::async_trait;
use teramind_ipc::client::{IpcClient as _, StreamClient};
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::rpc_transport::{RpcError, RpcTransport};
use teramind_ipc::transport;

pub struct LocalIpcTransport {
    socket_path: std::path::PathBuf,
}

impl LocalIpcTransport {
    pub fn new(socket_path: std::path::PathBuf) -> Self {
        Self { socket_path }
    }
}

#[async_trait]
impl RpcTransport for LocalIpcTransport {
    async fn request(&self, req: Request) -> Result<Response, RpcError> {
        let stream = transport::connect(&self.socket_path)
            .await
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        let mut client = StreamClient::new(stream);
        client
            .request(req)
            .await
            .map_err(|e| RpcError::Other(e.to_string()))
    }
}
