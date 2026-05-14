//! FS watcher service. Owns one notify::RecommendedWatcher per unique
//! active-session cwd, refcounted by session_id. On filesystem events,
//! debounces per (cwd, rel_path) and dispatches a full
//! pre/post/diff/excerpts/attribution pipeline.

use crate::services::ignore_filter::IgnoreFilter;
use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::SessionId;
use time::OffsetDateTime;
use tokio::sync::{mpsc, Mutex};

/// One watcher per unique cwd. Refcounted by active session_ids; the
/// watcher is dropped when the last session in that cwd ends.
pub struct WatchRegistry {
    pub(crate) inner: Mutex<HashMap<PathBuf, WatchEntry>>,
    event_tx: mpsc::UnboundedSender<RawEvent>,
    /// Incremented whenever `notify` reports an error (lost slot, etc.).
    /// Wired to `IngestStats.fs_watcher_gaps` so `teramind status` can surface it.
    gaps_counter: Arc<std::sync::atomic::AtomicU64>,
}

pub(crate) struct WatchEntry {
    pub(crate) sessions: HashSet<SessionId>,
    watcher: RecommendedWatcher,
    filter: IgnoreFilter,
}

#[derive(Debug, Clone)]
pub struct RawEvent {
    pub cwd: PathBuf,
    pub abs_path: PathBuf,
    pub at: OffsetDateTime,
}

impl WatchRegistry {
    pub fn new(
        event_tx: mpsc::UnboundedSender<RawEvent>,
        gaps_counter: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            event_tx,
            gaps_counter,
        }
    }

    pub async fn register(&self, cwd: PathBuf, session: SessionId) -> anyhow::Result<()> {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(&cwd) {
            entry.sessions.insert(session);
            return Ok(());
        }
        let cwd_for_cb = cwd.clone();
        let tx = self.event_tx.clone();
        let filter = IgnoreFilter::for_root(&cwd);
        let filter_for_cb = filter.clone();
        let gaps = self.gaps_counter.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let ev = match res {
                Ok(ev) => ev,
                Err(_) => {
                    gaps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            };
            if !matches!(ev.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            for p in ev.paths {
                if filter_for_cb.is_ignored(&p) { continue; }
                let _ = tx.send(RawEvent {
                    cwd: cwd_for_cb.clone(),
                    abs_path: p,
                    at: OffsetDateTime::now_utc(),
                });
            }
        })?;
        watcher.watch(&cwd, RecursiveMode::Recursive)?;
        let mut sessions = HashSet::new();
        sessions.insert(session);
        g.insert(cwd, WatchEntry { sessions, watcher, filter });
        Ok(())
    }

    pub async fn unregister(&self, cwd: &Path, session: SessionId) {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(cwd) {
            entry.sessions.remove(&session);
            if entry.sessions.is_empty() {
                g.remove(cwd);
            }
        }
    }

    #[cfg(test)]
    pub async fn watched_count(&self) -> usize {
        self.inner.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_counter() -> Arc<std::sync::atomic::AtomicU64> {
        Arc::new(std::sync::atomic::AtomicU64::new(0))
    }

    #[tokio::test]
    async fn register_then_unregister_drops_watcher() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        let sid = SessionId::new();
        reg.register(dir.path().to_path_buf(), sid).await.unwrap();
        assert_eq!(reg.watched_count().await, 1);
        reg.unregister(dir.path(), sid).await;
        assert_eq!(reg.watched_count().await, 0);
    }

    #[tokio::test]
    async fn second_session_in_same_cwd_does_not_duplicate_watcher() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        let a = SessionId::new();
        let b = SessionId::new();
        reg.register(dir.path().to_path_buf(), a).await.unwrap();
        reg.register(dir.path().to_path_buf(), b).await.unwrap();
        assert_eq!(reg.watched_count().await, 1);
        reg.unregister(dir.path(), a).await;
        assert_eq!(reg.watched_count().await, 1); // b still holds it
        reg.unregister(dir.path(), b).await;
        assert_eq!(reg.watched_count().await, 0);
    }

    #[tokio::test]
    async fn modify_event_is_emitted_to_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "x").unwrap();
        reg.register(dir.path().to_path_buf(), SessionId::new()).await.unwrap();
        // Modify the file.
        std::fs::write(dir.path().join("a.rs"), "y").unwrap();
        // Wait briefly for notify to fire.
        let evt = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await.expect("timed out").expect("channel closed");
        assert!(evt.abs_path.ends_with("a.rs"), "got {:?}", evt.abs_path);
    }
}
