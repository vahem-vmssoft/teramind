//! Async session-summarizer worker. Polls `sessions_to_summarize`, builds
//! a structured digest, applies the Redactor, calls the active provider,
//! persists Markdown. Capture-safe: never blocks ingest or search.

use crate::services::summarize::digest;
use crate::services::summarize::prompts::SYSTEM_PROMPT;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teramind_core::redact::Redactor;
use teramind_core::summarize::{SummaryError, SummaryProvider};
use teramind_db::repos::WikiRepo;
use tracing::{debug, warn};

#[derive(Default)]
pub struct SummarizerStats {
    pub written: AtomicU64,
    pub skipped: AtomicU64,
    pub errors: AtomicU64,
    pub backlog: AtomicU64,
    pub last_filled_at_unix: AtomicU64,
    pub provider_unhealthy_since_unix: AtomicU64,
    pub input_tokens_total: AtomicU64,
    pub output_tokens_total: AtomicU64,
}

pub struct SummarizerWorker {
    pub stats: Arc<SummarizerStats>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct SummarizerDeps {
    pub repo: WikiRepo,
    pub provider: Arc<dyn SummaryProvider>,
    pub redactor: Arc<Redactor>,
    pub model: String,
    pub poll_interval: Duration,
    pub min_turns: u32,
    pub min_duration_secs: u64,
    pub input_char_budget: u32,
    pub output_token_budget: u32,
}

impl SummarizerWorker {
    pub fn spawn(deps: SummarizerDeps) -> Self {
        let stats = Arc::new(SummarizerStats::default());
        let s = stats.clone();
        let handle = tokio::spawn(async move { run_loop(deps, s).await; });
        Self { stats, handle }
    }
    pub fn abort(&self) { self.handle.abort(); }
}

async fn run_loop(deps: SummarizerDeps, stats: Arc<SummarizerStats>) {
    loop {
        tokio::time::sleep(deps.poll_interval).await;

        match deps.provider.health_check().await {
            Ok(_) => stats.provider_unhealthy_since_unix.store(0, Ordering::Relaxed),
            Err(e) => {
                let prev = stats.provider_unhealthy_since_unix.load(Ordering::Relaxed);
                if prev == 0 { stats.provider_unhealthy_since_unix.store(unix_now(), Ordering::Relaxed); }
                debug!(?e, "summary provider unhealthy");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        }

        if let Ok(b) = deps.repo.backlog(&deps.model).await {
            stats.backlog.store(b as u64, Ordering::Relaxed);
        }

        let candidates = match deps.repo.fetch_sessions_to_summarize(&deps.model, 1).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "fetch_sessions_to_summarize failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        if candidates.is_empty() { continue; }
        let s = &candidates[0];

        let snapshot = match deps.repo.load_snapshot(s.session_id).await {
            Ok(Some(snap)) => snap,
            Ok(None) => continue,           // session vanished between fetch and load (cascade delete)
            Err(e) => {
                warn!(error = %e, "load_snapshot failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let duration_secs = snapshot.duration_secs() as u64;
        if snapshot.turn_count() < deps.min_turns as usize || duration_secs < deps.min_duration_secs {
            let _ = deps.repo.mark_skipped(s.session_id, &deps.model).await;
            stats.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        let digest_md = digest::build(&snapshot, deps.input_char_budget as usize);
        let digest_md = deps.redactor.apply(&digest_md);

        match deps.provider.summarize(SYSTEM_PROMPT, &digest_md, deps.output_token_budget as usize).await {
            Ok(result) => {
                if let Err(e) = deps.repo.upsert(
                    s.session_id, &deps.model, &result.content,
                    result.input_tokens, result.output_tokens,
                ).await {
                    warn!(error = %e, "wiki upsert failed");
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                stats.written.fetch_add(1, Ordering::Relaxed);
                stats.last_filled_at_unix.store(unix_now(), Ordering::Relaxed);
                stats.input_tokens_total.fetch_add(result.input_tokens as u64, Ordering::Relaxed);
                stats.output_tokens_total.fetch_add(result.output_tokens as u64, Ordering::Relaxed);
                debug!(session_id = ?s.session_id, "summarizer wrote wiki page");
            }
            Err(SummaryError::Unhealthy(_)) => { continue; }
            Err(SummaryError::ModelNotFound(_)) => {
                warn!("summarizer: model not found; pausing worker until config changes");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
            Err(e) => {
                warn!(error = %e, "summarize failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    // Behavior is exercised end-to-end in §13. This module has no isolated
    // unit tests beyond a smoke compile check.
    use super::*;

    #[test]
    fn worker_handle_abort_compiles() {
        let _ = SummarizerWorker::abort;
    }
}
