use crate::services::ingest::{IngestService, IngestStats};
use crate::services::rpc_dispatch::RpcDeps;
use crate::services::search::BlendWeights;
use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use teramind_core::embed::EmbeddingProvider;
use teramind_ipc::proto::{Notify, Request, Response, StatusReport};
use teramind_ipc::server::{serve_connection, IpcServer};

#[async_trait::async_trait]
pub trait TeamShareSetter: Send + Sync {
    async fn write_and_signal(
        &self,
        cwd: &std::path::Path,
        session_id: Option<teramind_core::ids::SessionId>,
        share: bool,
        set_by: &str,
    ) -> anyhow::Result<()>;
}

pub struct DaemonIpcHandler {
    pub ingest: Arc<IngestService>,
    pub stats: Arc<IngestStats>,
    pub started: Instant,
    pub last_pg_bytes: std::sync::atomic::AtomicI64,
    pub last_jsonl_bytes: std::sync::atomic::AtomicI64,
    pub search_repo: teramind_db::repos::SearchRepo,
    pub jsonl_dir: std::path::PathBuf,
    pub embed_provider: Arc<dyn EmbeddingProvider>,
    pub embed_model: String,
    pub search_weights: BlendWeights,
    pub embed_stats: std::sync::Arc<crate::services::embedding_worker::EmbeddingStats>,
    pub pool: teramind_db::pool::DbPool,
    pub wiki_repo: teramind_db::repos::WikiRepo,
    pub summary_provider: std::sync::Arc<dyn teramind_core::summarize::SummaryProvider>,
    pub summary_model: String,
    pub summarizer_stats: std::sync::Arc<crate::services::summarizer_worker::SummarizerStats>,
    pub decision_cache: Option<std::sync::Arc<crate::services::decision_cache::DecisionCache>>,
    pub team_share_writer: Option<std::sync::Arc<dyn TeamShareSetter>>,
}

impl DaemonIpcHandler {
    fn rpc_deps(&self) -> RpcDeps {
        RpcDeps {
            pool: self.pool.clone(),
            search_repo: self.search_repo.clone(),
            wiki_repo: self.wiki_repo.clone(),
            embed_provider: self.embed_provider.clone(),
            embed_model: self.embed_model.clone(),
            search_weights: self.search_weights,
            summary_provider: self.summary_provider.clone(),
            summary_model: self.summary_model.clone(),
            jsonl_dir: self.jsonl_dir.clone(),
            event_bus: None,
        }
    }
}

#[async_trait]
impl IpcServer for DaemonIpcHandler {
    async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Status => {
                let healthy = self
                    .embed_stats
                    .provider_unhealthy_since_unix
                    .load(Ordering::Relaxed)
                    == 0;
                let backlog = self.embed_stats.backlog.load(Ordering::Relaxed) as i64;
                let last_filled = {
                    let v = self.embed_stats.last_filled_at_unix.load(Ordering::Relaxed);
                    if v == 0 {
                        None
                    } else {
                        Some(v)
                    }
                };
                let summary_healthy = self
                    .summarizer_stats
                    .provider_unhealthy_since_unix
                    .load(Ordering::Relaxed)
                    == 0;
                let summary_backlog = self.summarizer_stats.backlog.load(Ordering::Relaxed) as i64;
                let summary_written = self.summarizer_stats.written.load(Ordering::Relaxed);
                let summary_in = self
                    .summarizer_stats
                    .input_tokens_total
                    .load(Ordering::Relaxed);
                let summary_out = self
                    .summarizer_stats
                    .output_tokens_total
                    .load(Ordering::Relaxed);
                Response::Status(StatusReport {
                    uptime_seconds: self.started.elapsed().as_secs(),
                    pg_connected: true,
                    ingest_queue_depth: self.stats.queue_depth.load(Ordering::Relaxed) as u32,
                    ingest_drops_total: self.stats.drops.load(Ordering::Relaxed),
                    last_storage_pg_bytes: self.last_pg_bytes.load(Ordering::Relaxed),
                    last_storage_jsonl_bytes: self.last_jsonl_bytes.load(Ordering::Relaxed),
                    fs_watcher_gaps_total: self.stats.fs_watcher_gaps.load(Ordering::Relaxed),
                    embedding_provider: Some(self.embed_model.clone()),
                    embedding_healthy: Some(healthy),
                    embedding_backlog: Some(backlog),
                    embedding_last_filled_unix: last_filled,
                    summary_provider: Some(self.summary_model.clone()),
                    summary_healthy: Some(summary_healthy),
                    summary_backlog: Some(summary_backlog),
                    summary_written_total: Some(summary_written),
                    summary_input_tokens_total: Some(summary_in),
                    summary_output_tokens_total: Some(summary_out),
                })
            }
            Request::Ping => Response::Pong,
            Request::Shutdown => Response::Ok,
            req @ (Request::Search(_)
            | Request::Recall(_)
            | Request::AutoRecall(_)
            | Request::SaveSkill(_)
            | Request::WikiLookup { .. }) => {
                crate::services::rpc_dispatch::dispatch(&self.rpc_deps(), req, None).await
            }
            Request::TeamShareSet {
                session_id,
                cwd,
                scope: _,
                share,
            } => {
                let Some(writer) = self.team_share_writer.as_ref() else {
                    return Response::Error("team mode not configured".into());
                };
                let sid = session_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(teramind_core::ids::SessionId);
                let cwd_path = std::path::PathBuf::from(&cwd);
                match writer.write_and_signal(&cwd_path, sid, share, "user").await {
                    Ok(()) => Response::Ok,
                    Err(e) => Response::Error(e.to_string()),
                }
            }
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
