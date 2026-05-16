use crate::services::ingest::{IngestService, IngestStats};
use crate::services::search::BlendWeights;
use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use teramind_core::embed::EmbeddingProvider;
use teramind_ipc::proto::{Notify, Request, Response, StatusReport};
use teramind_ipc::server::{serve_connection, IpcServer};

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
}

#[async_trait]
impl IpcServer for DaemonIpcHandler {
    async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Status => {
                let healthy = self.embed_stats.provider_unhealthy_since_unix.load(Ordering::Relaxed) == 0;
                let backlog = self.embed_stats.backlog.load(Ordering::Relaxed) as i64;
                let last_filled = {
                    let v = self.embed_stats.last_filled_at_unix.load(Ordering::Relaxed);
                    if v == 0 { None } else { Some(v) }
                };
                let summary_healthy = self.summarizer_stats.provider_unhealthy_since_unix.load(Ordering::Relaxed) == 0;
                let summary_backlog = self.summarizer_stats.backlog.load(Ordering::Relaxed) as i64;
                let summary_written = self.summarizer_stats.written.load(Ordering::Relaxed);
                let summary_in = self.summarizer_stats.input_tokens_total.load(Ordering::Relaxed);
                let summary_out = self.summarizer_stats.output_tokens_total.load(Ordering::Relaxed);
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
            Request::Search(r) => {
                let out = crate::services::search::do_search_with_fallback(
                    &self.search_repo,
                    &self.jsonl_dir,
                    Some(self.embed_provider.clone()),
                    &self.embed_model,
                    self.search_weights,
                    &r,
                ).await;
                Response::SearchResults(teramind_core::types::SearchResults {
                    hits: out.hits, degraded: out.degraded, took_ms: out.took_ms,
                })
            }
            Request::Recall(r) => {
                match crate::services::search::do_recall(&self.search_repo, &r).await {
                    Ok(out) => Response::SearchResults(teramind_core::types::SearchResults {
                        hits: out.hits, degraded: out.degraded, took_ms: out.took_ms,
                    }),
                    Err(e) => Response::Error(format!("recall failed: {e}")),
                }
            }
            Request::AutoRecall(r) => {
                match crate::services::search::do_auto_recall(&self.search_repo, &r).await {
                    Ok(md) => Response::AutoRecallDigest { markdown: md, degraded: false },
                    Err(_) => Response::AutoRecallDigest { markdown: String::new(), degraded: true },
                }
            }
            Request::SaveSkill(r) => {
                match self.search_repo.upsert_skill(&r).await {
                    Ok(s) => Response::SkillRef(s),
                    Err(e) => Response::Error(format!("save_skill failed: {e}")),
                }
            }
            Request::WikiLookup { session_id, cwd } => {
                let result: anyhow::Result<Option<teramind_db::repos::WikiPage>> = async {
                    if let Some(sid_str) = session_id {
                        let sid = teramind_core::ids::SessionId(uuid::Uuid::parse_str(&sid_str)?);
                        let p = self.wiki_repo.get_for_session(sid, &self.summary_model).await?;
                        Ok(p)
                    } else if let Some(cwd) = cwd {
                        let p = self.wiki_repo.latest_for_cwd(&cwd).await?;
                        Ok(p)
                    } else {
                        Ok(None)
                    }
                }.await;
                match result {
                    Ok(Some(p)) => {
                        let session_cwd: String = sqlx::query_scalar("SELECT cwd FROM sessions WHERE id = $1")
                            .bind(p.session_id.0)
                            .fetch_one(self.pool.pg())
                            .await
                            .unwrap_or_default();
                        Response::WikiPage {
                            session_id: p.session_id.0.to_string(),
                            cwd: session_cwd,
                            model: p.model,
                            content: p.content,
                            generated_at: p.generated_at,
                        }
                    }
                    Ok(None) => Response::WikiNotFound,
                    Err(e) => Response::Error(format!("wiki lookup failed: {e}")),
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
