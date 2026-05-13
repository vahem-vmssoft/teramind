use crate::services::ingest::{IngestService, IngestStats};
use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use teramind_ipc::proto::{Notify, Request, Response, StatusReport};
use teramind_ipc::server::{serve_connection, IpcServer};

pub struct DaemonIpcHandler {
    pub ingest: Arc<IngestService>,
    pub stats: Arc<IngestStats>,
    pub started: Instant,
    pub last_pg_bytes: std::sync::atomic::AtomicI64,
    pub last_jsonl_bytes: std::sync::atomic::AtomicI64,
}

#[async_trait]
impl IpcServer for DaemonIpcHandler {
    async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Status => Response::Status(StatusReport {
                uptime_seconds: self.started.elapsed().as_secs(),
                pg_connected: true,
                ingest_queue_depth: self.stats.queue_depth.load(Ordering::Relaxed) as u32,
                ingest_drops_total: self.stats.drops.load(Ordering::Relaxed),
                last_storage_pg_bytes: self.last_pg_bytes.load(Ordering::Relaxed),
                last_storage_jsonl_bytes: self.last_jsonl_bytes.load(Ordering::Relaxed),
            }),
            Request::Ping => Response::Pong,
            Request::Shutdown => Response::Ok,
        }
    }
    async fn handle_notify(&self, n: Notify) {
        match n {
            Notify::Ingest(env) => {
                let _ = self.ingest.try_enqueue(env);
            }
        }
    }
}

pub async fn run_accept_loop<L>(listener: L, handler: Arc<DaemonIpcHandler>) -> anyhow::Result<()>
where
    L: AcceptStream + Send + 'static,
{
    loop {
        let stream = listener.accept_stream().await?;
        let h = handler.clone();
        tokio::spawn(async move {
            let _ = serve_connection(stream, h).await;
        });
    }
}

#[async_trait::async_trait]
pub trait AcceptStream {
    type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static;
    async fn accept_stream(&self) -> std::io::Result<Self::Stream>;
}

#[cfg(unix)]
#[async_trait::async_trait]
impl AcceptStream for tokio::net::UnixListener {
    type Stream = tokio::net::UnixStream;
    async fn accept_stream(&self) -> std::io::Result<Self::Stream> {
        let (s, _) = self.accept().await?;
        Ok(s)
    }
}
