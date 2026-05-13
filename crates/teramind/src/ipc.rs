use std::time::Duration;
use teramind_ipc::client::{IpcClient, StreamClient};
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::transport::{connect, default_socket_path};

pub async fn request(req: Request, deadline_ms: u64) -> anyhow::Result<Response> {
    let path = default_socket_path();
    let stream = tokio::time::timeout(Duration::from_millis(deadline_ms), connect(&path))
        .await
        .map_err(|_| anyhow::anyhow!("daemon connect timed out"))??;
    let mut client = StreamClient::new(stream);
    Ok(client.request(req).await?)
}
