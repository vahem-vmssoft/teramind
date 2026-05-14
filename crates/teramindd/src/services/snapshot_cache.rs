//! In-memory map of (cwd, rel_path) -> last-seen file content.
//!
//! The FS watcher stores post-content here so the NEXT modification of
//! the same file has accurate pre-content. Entries older than the
//! configured TTL are evicted on insert.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SnapshotCache {
    inner: Arc<Mutex<HashMap<(PathBuf, String), Entry>>>,
    ttl: time::Duration,
}

#[derive(Clone)]
struct Entry {
    content: String,
    stored_at: OffsetDateTime,
}

impl SnapshotCache {
    pub fn new(ttl: time::Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub async fn get(&self, cwd: &PathBuf, rel_path: &str) -> Option<String> {
        let m = self.inner.lock().await;
        m.get(&(cwd.clone(), rel_path.to_string()))
            .map(|e| e.content.clone())
    }

    pub async fn put(&self, cwd: PathBuf, rel_path: String, content: String) {
        let now = OffsetDateTime::now_utc();
        let mut m = self.inner.lock().await;
        // Evict stale entries.
        m.retain(|_, e| now - e.stored_at < self.ttl);
        m.insert((cwd, rel_path), Entry { content, stored_at: now });
    }

    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }
}

use std::path::Path;

/// Resolve pre-content for (cwd, rel_path) using cache -> git index -> empty string.
pub async fn resolve_pre_content(
    cache: &SnapshotCache,
    cwd: &Path,
    rel_path: &str,
) -> String {
    if let Some(s) = cache.get(&cwd.to_path_buf(), rel_path).await {
        return s;
    }
    if let Some(s) = crate::services::git_index::show_index(cwd, rel_path).await {
        return s;
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn roundtrip_put_get() {
        let c = SnapshotCache::new(time::Duration::seconds(60));
        let cwd = PathBuf::from("/p");
        c.put(cwd.clone(), "a.rs".into(), "fn a(){}".into()).await;
        let got = c.get(&cwd, "a.rs").await;
        assert_eq!(got.as_deref(), Some("fn a(){}"));
    }

    #[tokio::test]
    async fn ttl_evicts_old_entries_on_next_put() {
        let c = SnapshotCache::new(time::Duration::milliseconds(50));
        let cwd = PathBuf::from("/p");
        c.put(cwd.clone(), "a".into(), "x".into()).await;
        assert_eq!(c.len().await, 1);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        c.put(cwd.clone(), "b".into(), "y".into()).await;
        // Old entry should have been evicted; only "b" remains.
        assert_eq!(c.len().await, 1);
        assert!(c.get(&cwd, "a").await.is_none());
        assert_eq!(c.get(&cwd, "b").await.as_deref(), Some("y"));
    }

    #[tokio::test]
    async fn resolve_pre_content_returns_cache_first() {
        let c = SnapshotCache::new(time::Duration::seconds(60));
        let cwd = PathBuf::from("/nonexistent-no-git");
        c.put(cwd.clone(), "a.rs".into(), "CACHED".into()).await;
        let s = resolve_pre_content(&c, &cwd, "a.rs").await;
        assert_eq!(s, "CACHED");
    }

    #[tokio::test]
    async fn resolve_pre_content_falls_back_to_empty_string_when_no_git() {
        let c = SnapshotCache::new(time::Duration::seconds(60));
        let dir = tempfile::tempdir().unwrap();
        let s = resolve_pre_content(&c, &dir.path().to_path_buf(), "ghost.rs").await;
        assert_eq!(s, "");
    }
}
