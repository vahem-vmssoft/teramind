//! Async embedding worker. Polls `traces_to_embed`, redacts, calls the
//! provider, persists vectors. Capture-safe: never blocks ingest or search.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teramind_core::embed::EmbeddingProvider;
use teramind_core::redact::Redactor;
use teramind_db::repos::EmbeddingRepo;
use tracing::{debug, warn};

#[derive(Default)]
pub struct EmbeddingStats {
    pub written: AtomicU64,
    pub errors: AtomicU64,
    pub backlog: AtomicU64,
    pub last_filled_at_unix: AtomicU64,
    pub provider_unhealthy_since_unix: AtomicU64,
}

pub struct EmbeddingWorker {
    pub stats: Arc<EmbeddingStats>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct EmbeddingWorkerDeps {
    pub repo: EmbeddingRepo,
    pub provider: Arc<dyn EmbeddingProvider>,
    pub redactor: Arc<Redactor>,
    pub model: String,
    pub poll_interval: Duration,
    pub batch_size: u32,
}

impl EmbeddingWorker {
    pub fn spawn(deps: EmbeddingWorkerDeps) -> Self {
        let stats = Arc::new(EmbeddingStats::default());
        let s = stats.clone();
        let handle = tokio::spawn(async move {
            run_loop(deps, s).await;
        });
        Self { stats, handle }
    }

    pub fn abort(&self) { self.handle.abort(); }
}

async fn run_loop(deps: EmbeddingWorkerDeps, stats: Arc<EmbeddingStats>) {
    loop {
        tokio::time::sleep(deps.poll_interval).await;
        match deps.provider.health_check().await {
            Ok(_) => stats.provider_unhealthy_since_unix.store(0, Ordering::Relaxed),
            Err(e) => {
                let prev = stats.provider_unhealthy_since_unix.load(Ordering::Relaxed);
                if prev == 0 { stats.provider_unhealthy_since_unix.store(unix_now(), Ordering::Relaxed); }
                debug!(?e, "embedding provider unhealthy");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        }

        if let Ok(b) = deps.repo.backlog(&deps.model).await {
            stats.backlog.store(b as u64, Ordering::Relaxed);
        }

        let rows = match deps.repo.fetch_to_embed(&deps.model, deps.batch_size).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "fetch_to_embed failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        if rows.is_empty() { continue; }

        let texts: Vec<String> = rows.iter()
            .map(|r| truncate_chars(&deps.redactor.apply(&r.text), deps.provider.max_tokens() * 4))
            .collect();

        let vectors = match embed_with_bisect(deps.provider.as_ref(), &texts).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "embed failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let dim = deps.provider.dimension() as i32;
        match deps.repo.bulk_insert(&rows, &deps.model, dim, &vectors).await {
            Ok(n) => {
                stats.written.fetch_add(n as u64, Ordering::Relaxed);
                stats.last_filled_at_unix.store(unix_now(), Ordering::Relaxed);
                debug!(written = n, "embedding_worker wrote batch");
            }
            Err(e) => {
                warn!(error = %e, "bulk_insert failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

async fn embed_with_bisect(
    provider: &dyn EmbeddingProvider,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, teramind_core::embed::EmbedError> {
    embed_with_bisect_depth(provider, texts, 0).await
}

#[async_recursion::async_recursion]
async fn embed_with_bisect_depth(
    provider: &(dyn EmbeddingProvider + Send + Sync),
    texts: &[String],
    depth: u8,
) -> Result<Vec<Vec<f32>>, teramind_core::embed::EmbedError> {
    match provider.embed(texts).await {
        Ok(v) => Ok(v),
        Err(e) if e.is_size_exceeded() && depth < 4 && texts.len() > 1 => {
            let mid = texts.len() / 2;
            let left  = embed_with_bisect_depth(provider, &texts[..mid], depth + 1).await?;
            let right = embed_with_bisect_depth(provider, &texts[mid..], depth + 1).await?;
            Ok([left, right].concat())
        }
        Err(e) => Err(e),
    }
}

fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes { return s.to_string(); }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    s[..end].to_string()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::embed::{DistanceMetric, EmbedError, ProviderKind};
    use async_trait::async_trait;

    struct MockProvider {
        dim: usize,
        fail_oversize_at: Option<usize>,
    }

    #[async_trait]
    impl EmbeddingProvider for MockProvider {
        fn kind(&self) -> ProviderKind { ProviderKind::Fastembed }
        fn model_id(&self) -> &str { "mock" }
        fn dimension(&self) -> usize { self.dim }
        fn max_tokens(&self) -> usize { 8192 }
        fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }
        async fn health_check(&self) -> Result<(), EmbedError> { Ok(()) }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            if let Some(thresh) = self.fail_oversize_at {
                if texts.len() > thresh {
                    return Err(EmbedError::SizeExceeded(format!("batch={}", texts.len())));
                }
            }
            Ok(texts.iter().map(|t| {
                let mut v = vec![0.0f32; self.dim];
                v[0] = t.len() as f32;
                v
            }).collect())
        }
    }

    #[test]
    fn truncate_chars_respects_codepoint_boundary() {
        let s = "héllo";
        let t = truncate_chars(s, 2);
        assert!(t == "h" || t == "hé");
        assert!(s.starts_with(&t));
    }

    #[tokio::test]
    async fn embed_with_bisect_recurses_on_size_exceeded() {
        let p = MockProvider { dim: 4, fail_oversize_at: Some(2) };
        let texts: Vec<String> = (0..4).map(|i| format!("text{i}")).collect();
        let vectors = embed_with_bisect(&p, &texts).await.expect("should split");
        assert_eq!(vectors.len(), 4);
    }

    #[tokio::test]
    async fn embed_with_bisect_gives_up_on_single_item_size_failure() {
        let p = MockProvider { dim: 4, fail_oversize_at: Some(0) };
        let texts: Vec<String> = vec!["x".into()];
        let r = embed_with_bisect(&p, &texts).await;
        assert!(r.is_err(), "single-item batch can't bisect further");
    }
}
