//! Daily sweep of orphan embeddings (rows whose parent turn/file_diff
//! was cascade-deleted). Runs in the background; never blocks anything.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teramind_db::repos::EmbeddingRepo;
use tracing::{info, warn};

pub struct OrphanSweeper {
    pub deleted: Arc<AtomicU64>,
    handle: tokio::task::JoinHandle<()>,
}

impl OrphanSweeper {
    pub fn spawn(repo: EmbeddingRepo, interval: Duration) -> Self {
        let deleted = Arc::new(AtomicU64::new(0));
        let d = deleted.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match repo.sweep_orphans().await {
                    Ok(n) => {
                        if n > 0 { info!(deleted = n, "orphan_sweeper removed embeddings"); }
                        d.fetch_add(n, Ordering::Relaxed);
                    }
                    Err(e) => warn!(error = %e, "orphan_sweeper sweep failed"),
                }
            }
        });
        Self { deleted, handle }
    }

    pub fn abort(&self) { self.handle.abort(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_compiles() {
        // sweep behavior is exercised in §15 L3 tests; this is a smoke compile check.
        let _ = OrphanSweeper::abort;
    }
}
